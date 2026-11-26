import AppKit
import Foundation
import NaturalLanguage
import SwiftUI
import Translation

private typealias NamiTranslationCallback = @convention(c) (
    UnsafeMutableRawPointer?,
    UnsafePointer<CChar>?
) -> Void

private struct CallbackBox: @unchecked Sendable {
    let callback: NamiTranslationCallback
    let context: UnsafeMutableRawPointer?

    func invoke(json: String) {
        json.withCString { pointer in
            callback(context, pointer)
        }
    }
}

private struct BridgeItem: Encodable {
    let text: String
    let detectedLanguage: String?
    let score: Double?
}

private struct BridgeResponse: Encodable {
    let ok: Bool
    let items: [BridgeItem]?
    let error: String?
    let code: String?
}

private let supportedNLLanguages: [NLLanguage] = [
    .arabic,
    .simplifiedChinese,
    .traditionalChinese,
    .dutch,
    .english,
    .french,
    .german,
    .hindi,
    .indonesian,
    .italian,
    .japanese,
    .korean,
    .polish,
    .portuguese,
    .russian,
    .spanish,
    .thai,
    .turkish,
    .ukrainian,
    .vietnamese,
]

private func encodeResponse(_ response: BridgeResponse) -> String {
    let encoder = JSONEncoder()
    guard let data = try? encoder.encode(response),
          let string = String(data: data, encoding: .utf8)
    else {
        return "{\"ok\":false,\"error\":\"Apple Translate returned an unreadable response.\",\"code\":\"failed\"}"
    }
    return string
}

private func nlLanguageToAppleIdentifier(_ language: NLLanguage) -> String? {
    switch language {
    case .arabic: return "ar-AE"
    case .simplifiedChinese: return "zh-Hans"
    case .traditionalChinese: return "zh-Hant"
    case .dutch: return "nl"
    case .english: return "en"
    case .french: return "fr"
    case .german: return "de"
    case .hindi: return "hi"
    case .indonesian: return "id"
    case .italian: return "it"
    case .japanese: return "ja"
    case .korean: return "ko"
    case .polish: return "pl"
    case .portuguese: return "pt"
    case .russian: return "ru"
    case .spanish: return "es"
    case .thai: return "th"
    case .turkish: return "tr"
    case .ukrainian: return "uk"
    case .vietnamese: return "vi"
    default: return nil
    }
}

private func canonicalLanguageGroup(_ identifier: String) -> String {
    let normalized = identifier.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    switch normalized {
    case "zh", "zh-cn", "zh-hans", "zh-chs":
        return "zh-hans"
    case "zh-tw", "zh-hant", "zh-hk", "zh-mo", "zh-cht":
        return "zh-hant"
    case "ar", "ar-ae":
        return "ar"
    case "en", "en-us":
        return "en"
    case "en-gb":
        return "en-gb"
    case "pt", "pt-br":
        return "pt"
    default:
        return normalized
    }
}

private func sameLanguage(_ lhs: String, _ rhs: String) -> Bool {
    canonicalLanguageGroup(lhs) == canonicalLanguageGroup(rhs)
}

private func detectLanguage(for text: String) -> (identifier: String, score: Double?)? {
    let recognizer = NLLanguageRecognizer()
    recognizer.languageConstraints = supportedNLLanguages
    recognizer.processString(text.replacingOccurrences(of: #"\[NAMIX*[0-9]+\]"#, with: " ", options: .regularExpression))
    let hypotheses = recognizer.languageHypotheses(withMaximum: 3)
    if let best = hypotheses.max(by: { $0.value < $1.value }),
       let id = nlLanguageToAppleIdentifier(best.key),
       best.value >= 0.25 {
        return (id, min(1.0, max(0.0, best.value)))
    }
    return nil
}

private final class AppleTranslationRegistry: @unchecked Sendable {
    static let shared = AppleTranslationRegistry()

    private let lock = NSLock()
    private var tasks: [UInt64: Task<Void, Never>] = [:]
    private var cancelHandlers: [UInt64: () -> Void] = [:]
    private var cancelledIds: Set<UInt64> = []

    func start(requestId: UInt64, operation: @escaping @Sendable () async -> Void) {
        lock.lock()
        tasks[requestId] = Task.detached(priority: .userInitiated, operation: operation)
        lock.unlock()
    }

    func setCancelHandler(requestId: UInt64, handler: @escaping () -> Void) {
        var shouldRunNow = false
        lock.lock()
        if cancelledIds.contains(requestId) || tasks[requestId] == nil {
            shouldRunNow = true
        } else {
            cancelHandlers[requestId] = handler
        }
        lock.unlock()
        if shouldRunNow {
            handler()
        }
    }

    func isActive(requestId: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return tasks[requestId] != nil && !cancelledIds.contains(requestId)
    }

    func finish(requestId: UInt64) {
        lock.lock()
        tasks.removeValue(forKey: requestId)
        cancelHandlers.removeValue(forKey: requestId)
        cancelledIds.remove(requestId)
        lock.unlock()
    }

    func cancel(requestId: UInt64) {
        lock.lock()
        guard let task = tasks[requestId], cancelledIds.insert(requestId).inserted else {
            lock.unlock()
            return
        }
        let handler = cancelHandlers.removeValue(forKey: requestId)
        lock.unlock()
        task.cancel()
        handler?()
    }
}

private enum AppleBridgeError: Error {
    case cancelled
    case notInstalled
    case unsupportedPair
    case unableToIdentify
    case failed(String)

    var response: BridgeResponse {
        switch self {
        case .cancelled:
            return BridgeResponse(ok: false, items: nil, error: "cancelled", code: "cancelled")
        case .notInstalled:
            return BridgeResponse(
                ok: false,
                items: nil,
                error: "Apple Translate language pack is not downloaded. Download it in System Settings → General → Language & Region → Translation Languages.",
                code: "not_installed"
            )
        case .unsupportedPair:
            return BridgeResponse(
                ok: false,
                items: nil,
                error: "This language pair is not supported by Apple Translate.",
                code: "unsupported"
            )
        case .unableToIdentify:
            return BridgeResponse(
                ok: false,
                items: nil,
                error: "Apple Translate could not detect the source language. Choose a source language manually.",
                code: "unable_to_identify"
            )
        case .failed(let message):
            return BridgeResponse(ok: false, items: nil, error: message, code: "failed")
        }
    }
}

@available(macOS 15.0, *)
private func classifyError(_ error: Error) -> AppleBridgeError {
    if let bridgeError = error as? AppleBridgeError {
        return bridgeError
    }
    if error is CancellationError {
        return .cancelled
    }
    #if compiler(>=6.2)
    if #available(macOS 26.0, *) {
        if TranslationError.alreadyCancelled ~= error {
            return .cancelled
        }
        if TranslationError.notInstalled ~= error {
            return .notInstalled
        }
    }
    #endif
    if TranslationError.unsupportedSourceLanguage ~= error
        || TranslationError.unsupportedTargetLanguage ~= error
        || TranslationError.unsupportedLanguagePairing ~= error {
        return .unsupportedPair
    }
    if TranslationError.unableToIdentifyLanguage ~= error {
        return .unableToIdentify
    }
    let description = error.localizedDescription.trimmingCharacters(in: .whitespacesAndNewlines)
    if description.isEmpty {
        return .failed("Apple Translate failed. Try again.")
    }
    let lowered = description.lowercased()
    if lowered.contains("cancel") {
        return .cancelled
    }
    if lowered.contains("offline") || lowered.contains("not installed") || lowered.contains("download") {
        return .notInstalled
    }
    return .failed(description)
}

@available(macOS 15.0, *)
private final class PassthroughHostingView<Content: View>: NSHostingView<Content> {
    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }
}

@available(macOS 15.0, *)
private struct HostedJob {
    let requestId: UInt64
    let source: Locale.Language?
    let target: Locale.Language
    let texts: [String]
    let completion: (Result<[(text: String, detectedLanguage: String)], AppleBridgeError>) -> Void
}

@available(macOS 15.0, *)
@MainActor
private final class AppleTranslationHostModel: ObservableObject {
    @Published var configuration: TranslationSession.Configuration?

    private var queue: [HostedJob] = []
    private var activeJob: HostedJob?
    private var hostingView: PassthroughHostingView<AppleTranslationHostView>?

    var activeRequestId: UInt64? { activeJob?.requestId }

    func enqueue(_ job: HostedJob) {
        queue.append(job)
        pumpIfIdle()
    }

    func cancel(requestId: UInt64) {
        if let index = queue.firstIndex(where: { $0.requestId == requestId }) {
            let removed = queue.remove(at: index)
            removed.completion(.failure(.cancelled))
            return
        }
        if let current = activeJob, current.requestId == requestId {
            activeJob = nil
            configuration = nil
            current.completion(.failure(.cancelled))
            pumpIfIdle()
        }
    }

    private func attachToActiveWindowIfPossible() -> Bool {
        let candidate = NSApp.keyWindow
            ?? NSApp.mainWindow
            ?? NSApp.windows.first(where: { $0.isVisible && $0.canBecomeKey })
        guard let window = candidate, let contentView = window.contentView else {
            return false
        }
        if let existing = hostingView {
            if existing.superview !== contentView {
                existing.removeFromSuperview()
                existing.frame = NSRect(x: 0, y: 0, width: 1, height: 1)
                contentView.addSubview(existing)
            }
        } else {
            let view = PassthroughHostingView(rootView: AppleTranslationHostView(model: self))
            view.frame = NSRect(x: 0, y: 0, width: 1, height: 1)
            view.translatesAutoresizingMaskIntoConstraints = true
            contentView.addSubview(view)
            hostingView = view
        }
        return true
    }

    private func pumpIfIdle() {
        guard activeJob == nil, !queue.isEmpty else { return }
        let next = queue.removeFirst()
        guard attachToActiveWindowIfPossible() else {
            next.completion(.failure(.failed("Apple Translate requires an active Nami window on this macOS version.")))
            pumpIfIdle()
            return
        }
        activeJob = next
        configuration = TranslationSession.Configuration(source: next.source, target: next.target)
    }

    func executeActive(requestId: UInt64?, with session: TranslationSession) async {
        guard let job = activeJob, requestId == job.requestId else { return }
        do {
            let translated = try await translateTexts(job.texts, with: session)
            finishActive(requestId: job.requestId, result: .success(translated))
        } catch {
            finishActive(requestId: job.requestId, result: .failure(classifyError(error)))
        }
    }

    private func finishActive(
        requestId: UInt64,
        result: Result<[(text: String, detectedLanguage: String)], AppleBridgeError>
    ) {
        guard let current = activeJob, current.requestId == requestId else { return }
        activeJob = nil
        configuration = nil
        current.completion(result)
        pumpIfIdle()
    }
}

@available(macOS 15.0, *)
private struct AppleTranslationHostView: View {
    @ObservedObject var model: AppleTranslationHostModel

    var body: some View {
        let requestId = model.activeRequestId
        Color.clear
            .frame(width: 1, height: 1)
            .allowsHitTesting(false)
            .translationTask(model.configuration) { session in
                await model.executeActive(requestId: requestId, with: session)
            }
            .id(requestId)
    }
}

@available(macOS 15.0, *)
@MainActor
private enum AppleTranslationHost {
    static let shared = AppleTranslationHostModel()
}

@available(macOS 15.0, *)
private func runHostedBatch(
    requestId: UInt64,
    source: Locale.Language?,
    target: Locale.Language,
    texts: [String]
) async throws -> [(text: String, detectedLanguage: String)] {
    try await withTaskCancellationHandler {
        try await withCheckedThrowingContinuation { continuation in
            Task { @MainActor in
                AppleTranslationRegistry.shared.setCancelHandler(requestId: requestId) {
                    Task { @MainActor in
                        AppleTranslationHost.shared.cancel(requestId: requestId)
                    }
                }
                guard AppleTranslationRegistry.shared.isActive(requestId: requestId) else {
                    continuation.resume(throwing: AppleBridgeError.cancelled)
                    return
                }
                AppleTranslationHost.shared.enqueue(
                    HostedJob(
                        requestId: requestId,
                        source: source,
                        target: target,
                        texts: texts,
                        completion: { result in
                            switch result {
                            case .success(let items):
                                continuation.resume(returning: items)
                            case .failure(let error):
                                continuation.resume(throwing: error)
                            }
                        }
                    )
                )
            }
        }
    } onCancel: {
        Task { @MainActor in
            AppleTranslationHost.shared.cancel(requestId: requestId)
        }
    }
}

@available(macOS 15.0, *)
private func translateGroup(
    requestId: UInt64,
    sourceIdentifier: String?,
    targetIdentifier: String,
    texts: [String]
) async throws -> [(text: String, detectedLanguage: String)] {
    if Task.isCancelled {
        throw AppleBridgeError.cancelled
    }
    if let sourceIdentifier, sameLanguage(sourceIdentifier, targetIdentifier) {
        return texts.map { (text: $0, detectedLanguage: sourceIdentifier) }
    }
    let targetLanguage = Locale.Language(identifier: targetIdentifier)
    let sourceLanguage = sourceIdentifier.map { Locale.Language(identifier: $0) }
    let availability = LanguageAvailability()
    if let sourceLanguage {
        try requireInstalled(await availability.status(from: sourceLanguage, to: targetLanguage))
    } else {
        for text in texts {
            try Task.checkCancellation()
            try requireInstalled(try await availability.status(for: text, to: targetLanguage))
        }
    }
    try Task.checkCancellation()
    #if compiler(>=6.2)
    if let sourceLanguage, #available(macOS 26.0, *) {
        let session = TranslationSession(installedSource: sourceLanguage, target: targetLanguage)
        AppleTranslationRegistry.shared.setCancelHandler(requestId: requestId) {
            session.cancel()
        }
        return try await translateTexts(texts, with: session)
    }
    #endif
    return try await runHostedBatch(requestId: requestId, source: sourceLanguage, target: targetLanguage, texts: texts)
}

@available(macOS 15.0, *)
private func requireInstalled(_ status: LanguageAvailability.Status) throws {
    switch status {
    case .installed: return
    case .supported: throw AppleBridgeError.notInstalled
    case .unsupported: throw AppleBridgeError.unsupportedPair
    @unknown default: throw AppleBridgeError.unsupportedPair
    }
}

@available(macOS 15.0, *)
private func translateTexts(
    _ texts: [String], with session: TranslationSession
) async throws -> [(text: String, detectedLanguage: String)] {
    let requests = texts.enumerated().map { index, text in
        TranslationSession.Request(sourceText: text, clientIdentifier: String(index))
    }
    let responses = try await session.translations(from: requests)
    let malformed = AppleBridgeError.failed("Apple Translate returned an incomplete result. Try again.")
    guard responses.count == requests.count else { throw malformed }
    var ordered = Array<(text: String, detectedLanguage: String)?>(repeating: nil, count: texts.count)
    for response in responses {
        guard let index = response.clientIdentifier.flatMap(Int.init),
              ordered.indices.contains(index), ordered[index] == nil else { throw malformed }
        ordered[index] = (response.targetText, response.sourceLanguage.minimalIdentifier)
    }
    return ordered.compactMap { $0 }
}

@available(macOS 15.0, *)
private func executeTranslation(
    requestId: UInt64,
    texts: [String],
    source: String,
    target: String
) async -> BridgeResponse {
    do {
        if texts.isEmpty {
            return BridgeResponse(ok: true, items: [], error: nil, code: nil)
        }
        if source != "auto" {
            let results = try await translateGroup(
                requestId: requestId,
                sourceIdentifier: source,
                targetIdentifier: target,
                texts: texts
            )
            let items = results.map { item in
                BridgeItem(text: item.text, detectedLanguage: nil, score: nil)
            }
            return BridgeResponse(ok: true, items: items, error: nil, code: nil)
        }

        // Auto-detect source language per segment while preserving original ordering.
        var groupedIndices: [String: [Int]] = [:]
        var groupOrder: [String] = []
        var scores = Array<Double?>(repeating: nil, count: texts.count)
        let unknownKey = "__auto__"

        for (index, text) in texts.enumerated() {
            if let detected = detectLanguage(for: text) {
                scores[index] = detected.score
                if groupedIndices[detected.identifier] == nil {
                    groupOrder.append(detected.identifier)
                }
                groupedIndices[detected.identifier, default: []].append(index)
            } else {
                if groupedIndices[unknownKey] == nil {
                    groupOrder.append(unknownKey)
                }
                groupedIndices[unknownKey, default: []].append(index)
            }
        }

        var output = Array<BridgeItem?>(repeating: nil, count: texts.count)
        for key in groupOrder {
            guard let indices = groupedIndices[key], !indices.isEmpty else { continue }
            let groupTexts = indices.map { texts[$0] }
            let groupSource = key == unknownKey ? nil : key
            let translated = try await translateGroup(
                requestId: requestId,
                sourceIdentifier: groupSource,
                targetIdentifier: target,
                texts: groupTexts
            )
            for (offset, originalIndex) in indices.enumerated() {
                let row = translated[offset]
                let detectedLang = row.detectedLanguage.isEmpty ? (groupSource ?? "und") : row.detectedLanguage
                output[originalIndex] = BridgeItem(
                    text: row.text,
                    detectedLanguage: detectedLang,
                    score: scores[originalIndex]
                )
            }
        }

        let finalized = output.enumerated().map { index, item in
            item ?? BridgeItem(text: texts[index], detectedLanguage: "und", score: nil)
        }
        return BridgeResponse(ok: true, items: finalized, error: nil, code: nil)
    } catch let bridgeError as AppleBridgeError {
        return bridgeError.response
    } catch {
        return classifyError(error).response
    }
}

@_cdecl("nami_apple_translation_available")
public func namiAppleTranslationAvailable() -> Bool {
    if #available(macOS 15.0, *) {
        return true
    }
    return false
}

@_cdecl("nami_apple_translation_cancel")
public func namiAppleTranslationCancel(_ requestId: UInt64) {
    AppleTranslationRegistry.shared.cancel(requestId: requestId)
}

@_cdecl("nami_apple_translation_translate")
public func namiAppleTranslationTranslate(
    _ requestId: UInt64,
    _ textsJson: UnsafePointer<CChar>,
    _ source: UnsafePointer<CChar>,
    _ target: UnsafePointer<CChar>,
    _ callback: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Void,
    _ context: UnsafeMutableRawPointer?
) {
    let box = CallbackBox(callback: callback, context: context)
    guard #available(macOS 15.0, *) else {
        box.invoke(json: encodeResponse(BridgeResponse(
            ok: false,
            items: nil,
            error: "Apple Translate requires macOS 15 or later.",
            code: "platform_unavailable"
        )))
        return
    }

    let rawJson = String(cString: textsJson)
    let sourceCode = String(cString: source)
    let targetCode = String(cString: target)

    guard let jsonData = rawJson.data(using: .utf8),
          let texts = try? JSONDecoder().decode([String].self, from: jsonData)
    else {
        box.invoke(json: encodeResponse(BridgeResponse(
            ok: false,
            items: nil,
            error: "Invalid Apple Translate request payload.",
            code: "invalid_argument"
        )))
        return
    }

    AppleTranslationRegistry.shared.start(requestId: requestId) {
        let response = await executeTranslation(
            requestId: requestId,
            texts: texts,
            source: sourceCode,
            target: targetCode
        )
        AppleTranslationRegistry.shared.finish(requestId: requestId)
        box.invoke(json: encodeResponse(response))
    }

}
