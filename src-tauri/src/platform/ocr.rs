// Cargo shares macOS framework dependencies across hosts; portable UI never calls Vision.
#[cfg(all(target_os = "macos", feature = "portable-ui"))]
use objc2_vision as _;

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
const MAX_PIXELS: u64 = 20_000_000;
#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
const MAX_DECODED_BYTES: u64 = 80 * 1024 * 1024;

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
fn prepare_vision_handler(
    bytes: &[u8],
) -> Result<objc2::rc::Retained<objc2_vision::VNImageRequestHandler>, String> {
    use objc2::AnyThread;
    use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_foundation::NSDictionary;
    use objc2_image_io::{
        kCGImagePropertyDepth, kCGImagePropertyOrientation, kCGImagePropertyPixelHeight,
        kCGImagePropertyPixelWidth, CGImagePropertyOrientation, CGImageSource, CGImageSourceStatus,
    };
    use objc2_vision::VNImageRequestHandler;

    if bytes.len() > MAX_ENCODED_BYTES {
        return Err("ocr_image_limit".into());
    }
    let data = CFData::from_bytes(bytes);
    // ImageIO metadata inspection does not decode the pixel buffer.
    let source = unsafe { CGImageSource::with_data(&data, None) }.ok_or("ocr_invalid_image")?;
    if unsafe { source.status() } != CGImageSourceStatus::StatusComplete
        || unsafe { source.count() } == 0
    {
        return Err("ocr_invalid_image".into());
    }
    let properties = unsafe { source.properties_at_index(0, None) }.ok_or("ocr_invalid_image")?;
    // ImageIO property dictionaries have CFString keys and CFType values.
    let properties: CFRetained<CFDictionary<CFString, CFType>> =
        unsafe { CFRetained::cast_unchecked(properties) };
    let number = |key: &CFString| {
        properties
            .get(key)
            .and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64))
    };
    let width = number(unsafe { kCGImagePropertyPixelWidth }).ok_or("ocr_invalid_image")?;
    let height = number(unsafe { kCGImagePropertyPixelHeight }).ok_or("ocr_invalid_image")?;
    let depth = number(unsafe { kCGImagePropertyDepth }).unwrap_or(8);
    validate_dimensions(width, height, depth)?;
    let orientation = number(unsafe { kCGImagePropertyOrientation }).unwrap_or(1);
    let orientation = match orientation {
        1 => CGImagePropertyOrientation::Up,
        2 => CGImagePropertyOrientation::UpMirrored,
        3 => CGImagePropertyOrientation::Down,
        4 => CGImagePropertyOrientation::DownMirrored,
        5 => CGImagePropertyOrientation::LeftMirrored,
        6 => CGImagePropertyOrientation::Right,
        7 => CGImagePropertyOrientation::RightMirrored,
        8 => CGImagePropertyOrientation::Left,
        _ => return Err("ocr_invalid_image".into()),
    };
    Ok(unsafe {
        VNImageRequestHandler::initWithData_orientation_options(
            VNImageRequestHandler::alloc(),
            data.as_ref(),
            orientation,
            &NSDictionary::new(),
        )
    })
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
// Vision performs synchronously; callers must use their bounded background worker.
fn recognize_image_text_in_process(bytes: &[u8]) -> Result<String, String> {
    use objc2::rc::autoreleasepool;
    use objc2_foundation::NSArray;
    use objc2_vision::{VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};

    autoreleasepool(|_| {
        let handler = prepare_vision_handler(bytes)?;
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        // Prefer CJK-aware recognition for mixed short text without dropping supported scripts.
        if let Ok(languages) = unsafe { request.supportedRecognitionLanguagesAndReturnError() } {
            let mut languages: Vec<_> = languages.iter().collect();
            languages.sort_by_key(|language| match language.to_string().as_str() {
                "zh-Hans" => 0,
                "en-US" => 1,
                _ => 2,
            });
            request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
            request.setAutomaticallyDetectsLanguage(false);
        } else {
            request.setAutomaticallyDetectsLanguage(true);
        }
        request.setMinimumTextHeight(0.0);
        let requests = NSArray::from_slice(&[request.as_ref() as &VNRequest]);
        handler
            .performRequests_error(&requests)
            .map_err(|_| "ocr_recognition_failed")?;
        let observations = request.results().ok_or("ocr_recognition_failed")?;
        Ok(observations
            .iter()
            .filter_map(|observation| observation.topCandidates(1).firstObject())
            .map(|candidate| candidate.string().to_string())
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"))
    })
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
fn detect_image_qr_codes_in_process(bytes: &[u8]) -> Result<Vec<String>, String> {
    use objc2::rc::autoreleasepool;
    use objc2_foundation::NSArray;
    use objc2_vision::{VNBarcodeSymbologyQR, VNDetectBarcodesRequest, VNRequest};

    autoreleasepool(|_| {
        let handler = prepare_vision_handler(bytes)?;
        let request = unsafe { VNDetectBarcodesRequest::new() };
        if let Some(qr) = unsafe { VNBarcodeSymbologyQR } {
            let symbologies = NSArray::from_slice(&[qr]);
            unsafe { request.setSymbologies(&symbologies) };
        }
        let requests = NSArray::from_slice(&[request.as_ref() as &VNRequest]);
        handler
            .performRequests_error(&requests)
            .map_err(|_| "ocr_recognition_failed")?;
        let Some(observations) = (unsafe { request.results() }) else {
            return Ok(Vec::new());
        };
        let mut results = Vec::new();
        for observation in observations.iter() {
            let Some(payload) = (unsafe { observation.payloadStringValue() }) else {
                continue;
            };
            let text = payload.to_string();
            if !text.trim().is_empty() && !results.contains(&text) {
                results.push(text);
                if results.len() >= 8 {
                    break;
                }
            }
        }
        Ok(results)
    })
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod worker {
    use super::{
        detect_image_qr_codes_in_process, recognize_image_text_in_process, MAX_ENCODED_BYTES,
    };
    use std::io::{self, Read, Write};
    #[cfg(not(test))]
    use std::{
        process::{Child, ChildStdin, ChildStdout, Command, Stdio},
        sync::{Arc, Condvar, Mutex, OnceLock},
        time::Duration,
    };

    pub(super) const WORKER_ARG: &str = "--nami-vision-worker";
    pub(super) const OP_OCR: u8 = 1;
    pub(super) const OP_QR: u8 = 2;
    const STATUS_OK: u8 = 0;
    const STATUS_ERR: u8 = 1;
    #[cfg(not(test))]
    const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
    #[cfg(not(test))]
    const IDLE_TIMEOUT: Duration = Duration::from_secs(5);

    #[cfg(not(test))]
    struct WorkerProcess {
        child: Child,
        stdin: Option<ChildStdin>,
        stdout: ChildStdout,
    }

    #[cfg(not(test))]
    impl WorkerProcess {
        fn spawn() -> Result<Self, String> {
            let exe = std::env::current_exe().map_err(|_| "ocr_recognition_failed")?;
            let mut child = Command::new(exe)
                .arg(WORKER_ARG)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "ocr_recognition_failed")?;
            let stdin = child.stdin.take().ok_or("ocr_recognition_failed")?;
            let stdout = child.stdout.take().ok_or("ocr_recognition_failed")?;
            Ok(Self {
                child,
                stdin: Some(stdin),
                stdout,
            })
        }

        fn call(&mut self, op: u8, bytes: &[u8]) -> Result<Result<Vec<u8>, String>, io::Error> {
            let stdin = self
                .stdin
                .as_mut()
                .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "closed"))?;
            write_frame(stdin, op, bytes)?;
            stdin.flush()?;
            let Some((status, body)) = read_frame(&mut self.stdout, MAX_RESPONSE_BYTES)? else {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "worker exited",
                ));
            };
            if status == STATUS_OK {
                Ok(Ok(body))
            } else {
                let code =
                    String::from_utf8(body).unwrap_or_else(|_| "ocr_recognition_failed".into());
                Ok(Err(code))
            }
        }

        fn shutdown(mut self) {
            self.stdin.take();
            for _ in 0..20 {
                match self.child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                    Err(_) => break,
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(not(test))]
    impl Drop for WorkerProcess {
        fn drop(&mut self) {
            self.stdin.take();
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
    }

    pub(super) fn write_frame(writer: &mut impl Write, tag: u8, body: &[u8]) -> io::Result<()> {
        let len = u32::try_from(body.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
        let mut header = [0u8; 5];
        header[0] = tag;
        header[1..5].copy_from_slice(&len.to_be_bytes());
        writer.write_all(&header)?;
        writer.write_all(body)
    }

    pub(super) fn read_frame(
        reader: &mut impl Read,
        limit: usize,
    ) -> io::Result<Option<(u8, Vec<u8>)>> {
        let mut first = [0u8; 1];
        match reader.read_exact(&mut first) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame exceeds limit",
            ));
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body)?;
        Ok(Some((first[0], body)))
    }

    #[cfg(not(test))]
    #[derive(Default)]
    struct PoolState {
        worker: Option<WorkerProcess>,
        generation: u64,
        busy: bool,
    }

    #[cfg(not(test))]
    struct WorkerPool {
        call_lock: Mutex<()>,
        state: Arc<(Mutex<PoolState>, Condvar)>,
    }

    #[cfg(not(test))]
    impl WorkerPool {
        fn new() -> Self {
            let state = Arc::new((Mutex::new(PoolState::default()), Condvar::new()));
            let reaper = Arc::clone(&state);
            let _ = std::thread::Builder::new()
                .name("nami-vision-reaper".into())
                .spawn(move || {
                    let (lock, condvar) = &*reaper;
                    let mut guard = lock.lock().unwrap();
                    loop {
                        while guard.worker.is_none() || guard.busy {
                            guard = condvar.wait(guard).unwrap();
                        }
                        let seen = guard.generation;
                        let (next, _) = condvar.wait_timeout(guard, IDLE_TIMEOUT).unwrap();
                        guard = next;
                        if !guard.busy && guard.generation == seen {
                            if let Some(worker) = guard.worker.take() {
                                drop(guard);
                                worker.shutdown();
                                guard = lock.lock().unwrap();
                            }
                        }
                    }
                });
            Self {
                call_lock: Mutex::new(()),
                state,
            }
        }

        fn execute(&self, op: u8, bytes: &[u8]) -> Result<Vec<u8>, String> {
            let _serial = self.call_lock.lock().unwrap();
            let (lock, condvar) = &*self.state;
            let mut active = {
                let mut guard = lock.lock().unwrap();
                guard.busy = true;
                guard.worker.take()
            };
            let mut result = None;
            for attempt in 0..2 {
                let mut process = match active.take() {
                    Some(existing) => existing,
                    None => WorkerProcess::spawn()?,
                };
                match process.call(op, bytes) {
                    Ok(response) => {
                        active = Some(process);
                        result = Some(response);
                        break;
                    }
                    Err(_) => {
                        process.shutdown();
                        if attempt == 1 {
                            result = Some(Err("ocr_recognition_failed".into()));
                        }
                    }
                }
            }
            {
                let mut guard = lock.lock().unwrap();
                guard.worker = active;
                guard.busy = false;
                guard.generation = guard.generation.wrapping_add(1);
                condvar.notify_all();
            }
            result.unwrap_or_else(|| Err("ocr_recognition_failed".into()))
        }

        fn shutdown(&self) {
            let (lock, condvar) = &*self.state;
            let worker = {
                let mut guard = lock.lock().unwrap();
                guard.generation = guard.generation.wrapping_add(1);
                condvar.notify_all();
                guard.worker.take()
            };
            if let Some(worker) = worker {
                worker.shutdown();
            }
        }
    }

    #[cfg(not(test))]
    fn pool() -> &'static WorkerPool {
        static POOL: OnceLock<WorkerPool> = OnceLock::new();
        POOL.get_or_init(WorkerPool::new)
    }

    #[cfg(not(test))]
    pub(super) fn call_worker(op: u8, bytes: &[u8]) -> Result<Vec<u8>, String> {
        if bytes.is_empty() {
            return Err("ocr_invalid_image".into());
        }
        if bytes.len() > MAX_ENCODED_BYTES {
            return Err("ocr_image_limit".into());
        }
        pool().execute(op, bytes)
    }

    pub(super) fn shutdown() {
        #[cfg(not(test))]
        pool().shutdown();
    }

    pub(super) fn run_stdio_loop(
        reader: &mut impl Read,
        writer: &mut impl Write,
    ) -> io::Result<()> {
        while let Some((op, bytes)) = read_frame(reader, MAX_ENCODED_BYTES)? {
            let outcome: Result<Vec<u8>, String> = match op {
                OP_OCR => recognize_image_text_in_process(&bytes).and_then(|text| {
                    serde_json::to_vec(&text).map_err(|_| "ocr_recognition_failed".into())
                }),
                OP_QR => detect_image_qr_codes_in_process(&bytes).and_then(|codes| {
                    serde_json::to_vec(&codes).map_err(|_| "ocr_recognition_failed".into())
                }),
                _ => Err("ocr_recognition_failed".into()),
            };
            match outcome {
                Ok(body) => write_frame(writer, STATUS_OK, &body)?,
                Err(code) => write_frame(writer, STATUS_ERR, code.as_bytes())?,
            }
            writer.flush()?;
        }
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub fn maybe_run_vision_worker() -> Option<i32> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == worker::WORKER_ARG)
    {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        let status = match worker::run_stdio_loop(&mut stdin, &mut stdout) {
            Ok(()) => 0,
            Err(_) => 1,
        };
        return Some(status);
    }
    None
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub fn shutdown_vision_worker() {
    worker::shutdown();
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub fn recognize_image_text(bytes: &[u8]) -> Result<String, String> {
    #[cfg(test)]
    {
        recognize_image_text_in_process(bytes)
    }
    #[cfg(not(test))]
    {
        let body = worker::call_worker(worker::OP_OCR, bytes)?;
        serde_json::from_slice(&body).map_err(|_| "ocr_recognition_failed".into())
    }
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub fn detect_image_qr_codes(bytes: &[u8]) -> Result<Vec<String>, String> {
    #[cfg(test)]
    {
        detect_image_qr_codes_in_process(bytes)
    }
    #[cfg(not(test))]
    {
        let body = worker::call_worker(worker::OP_QR, bytes)?;
        serde_json::from_slice(&body).map_err(|_| "ocr_recognition_failed".into())
    }
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub fn maybe_run_vision_worker() -> Option<i32> {
    None
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub fn shutdown_vision_worker() {}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub fn recognize_image_text(_bytes: &[u8]) -> Result<String, String> {
    Err("platform_unavailable".into())
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub fn detect_image_qr_codes(_bytes: &[u8]) -> Result<Vec<String>, String> {
    Err("platform_unavailable".into())
}

#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
fn validate_dimensions(width: i64, height: i64, depth: i64) -> Result<(), String> {
    if width <= 0 || height <= 0 || depth <= 0 {
        return Err("ocr_invalid_image".into());
    }
    let pixels = (width as u64).checked_mul(height as u64);
    let bytes = pixels.and_then(|pixels| pixels.checked_mul((depth as u64).div_ceil(8) * 4));
    if pixels.is_none_or(|pixels| pixels > MAX_PIXELS)
        || bytes.is_none_or(|bytes| bytes > MAX_DECODED_BYTES)
    {
        return Err("ocr_image_limit".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_reject_invalid_dimensions_overflow_and_large_high_depth_images() {
        assert_eq!(
            validate_dimensions(0, 12, 8),
            Err("ocr_invalid_image".into())
        );
        assert_eq!(
            validate_dimensions(12, -1, 8),
            Err("ocr_invalid_image".into())
        );
        assert_eq!(
            validate_dimensions(12, 12, 0),
            Err("ocr_invalid_image".into())
        );
        for size in [(5000, 5000, 8), (4000, 4000, 16), (i64::MAX, i64::MAX, 8)] {
            assert_eq!(
                validate_dimensions(size.0, size.1, size.2),
                Err("ocr_image_limit".into())
            );
        }
        assert!(validate_dimensions(4000, 4000, 8).is_ok());
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn vision_recognizes_synthetic_text_and_distinguishes_blank_corrupt_and_oversized_images() {
        let blank = bitmap(false);
        assert_eq!(recognize_image_text(&blank), Ok(String::new()));
        let text = recognize_image_text(&bitmap(true)).expect("synthetic text OCR succeeds");
        assert!(text.contains("HELLO") && text.contains("NAMI"), "{text:?}");
        assert_eq!(
            recognize_image_text(b"invalid image"),
            Err("ocr_invalid_image".into())
        );
        assert_eq!(recognize_image_text(&[]), Err("ocr_invalid_image".into()));
        assert_eq!(
            recognize_image_text(&vec![0; MAX_ENCODED_BYTES + 1]),
            Err("ocr_image_limit".into())
        );
        let mut oversized = blank;
        oversized[18..22].copy_from_slice(&5000i32.to_le_bytes());
        oversized[22..26].copy_from_slice(&5000i32.to_le_bytes());
        assert_eq!(
            recognize_image_text(&oversized),
            Err("ocr_image_limit".into())
        );
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn vision_detects_qr_code_and_handles_blank_and_corrupt_images() {
        let blank = bitmap(false);
        assert_eq!(detect_image_qr_codes(&blank), Ok(Vec::new()));
        assert_eq!(
            detect_image_qr_codes(b"invalid image"),
            Err("ocr_invalid_image".into())
        );
        let qr = synthetic_qr_png("https://example.test/nami-qr");
        let codes = detect_image_qr_codes(&qr).expect("synthetic QR detection succeeds");
        assert_eq!(codes, vec!["https://example.test/nami-qr"]);
    }

    #[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
    #[test]
    fn unsupported_host_never_attempts_recognition() {
        assert_eq!(
            recognize_image_text(b"image"),
            Err("platform_unavailable".into())
        );
        assert_eq!(
            detect_image_qr_codes(b"image"),
            Err("platform_unavailable".into())
        );
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn vision_worker_stdio_protocol_roundtrips_ocr_qr_and_errors() {
        let mut request = Vec::new();
        worker::write_frame(&mut request, worker::OP_OCR, &bitmap(true)).unwrap();
        worker::write_frame(
            &mut request,
            worker::OP_QR,
            &synthetic_qr_png("https://example.test/worker-qr"),
        )
        .unwrap();
        worker::write_frame(&mut request, worker::OP_OCR, b"corrupt").unwrap();

        let mut reader = std::io::Cursor::new(request);
        let mut response = Vec::new();
        worker::run_stdio_loop(&mut reader, &mut response).unwrap();

        let mut out = std::io::Cursor::new(response);
        let (status, body) = worker::read_frame(&mut out, 1024 * 1024).unwrap().unwrap();
        assert_eq!(status, 0);
        let text: String = serde_json::from_slice(&body).unwrap();
        assert!(text.contains("HELLO") && text.contains("NAMI"), "{text:?}");

        let (status, body) = worker::read_frame(&mut out, 1024 * 1024).unwrap().unwrap();
        assert_eq!(status, 0);
        let codes: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(codes, vec!["https://example.test/worker-qr"]);

        let (status, body) = worker::read_frame(&mut out, 1024 * 1024).unwrap().unwrap();
        assert_eq!(status, 1);
        assert_eq!(std::str::from_utf8(&body).unwrap(), "ocr_invalid_image");
        assert!(worker::read_frame(&mut out, 1024).unwrap().is_none());
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    fn synthetic_qr_png(payload: &str) -> Vec<u8> {
        use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
        use objc2_foundation::{ns_string, NSData};
        unsafe {
            let filter: Retained<AnyObject> =
                msg_send![class!(CIFilter), filterWithName: ns_string!("CIQRCodeGenerator")];
            let data = NSData::with_bytes(payload.as_bytes());
            let _: () = msg_send![&*filter, setValue: &*data, forKey: ns_string!("inputMessage")];
            let _: () = msg_send![
                &*filter,
                setValue: ns_string!("M"),
                forKey: ns_string!("inputCorrectionLevel")
            ];
            let output: Retained<AnyObject> = msg_send![&*filter, outputImage];
            let scale_filter: Retained<AnyObject> =
                msg_send![class!(CIFilter), filterWithName: ns_string!("CILanczosScaleTransform")];
            let _: () =
                msg_send![&*scale_filter, setValue: &*output, forKey: ns_string!("inputImage")];
            let scale: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: 8.0f64];
            let _: () =
                msg_send![&*scale_filter, setValue: &*scale, forKey: ns_string!("inputScale")];
            let scaled: Retained<AnyObject> = msg_send![&*scale_filter, outputImage];
            let rep: Retained<AnyObject> = msg_send![
                msg_send![class!(NSCIImageRep), alloc],
                initWithCIImage: &*scaled
            ];
            let width: isize = msg_send![&*rep, pixelsWide];
            let height: isize = msg_send![&*rep, pixelsHigh];
            let pad = 32usize;
            let total_w = (width.max(1) as usize) + pad * 2;
            let total_h = (height.max(1) as usize) + pad * 2;
            let pixel_bytes = total_w * total_h * 3;
            let mut bmp = vec![255u8; 54 + pixel_bytes];
            bmp[..54].fill(0);
            bmp[..2].copy_from_slice(b"BM");
            bmp[2..6].copy_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
            bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
            bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
            bmp[18..22].copy_from_slice(&(total_w as i32).to_le_bytes());
            bmp[22..26].copy_from_slice(&(total_h as i32).to_le_bytes());
            bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
            bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
            bmp[34..38].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
            let bmp_data = NSData::with_bytes(&bmp);
            let bitmap: Retained<AnyObject> =
                msg_send![class!(NSBitmapImageRep), imageRepWithData: &*bmp_data];
            let context: Retained<AnyObject> = msg_send![
                class!(NSGraphicsContext),
                graphicsContextWithBitmapImageRep: &*bitmap
            ];
            let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
            let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: &*context];
            let _: bool = msg_send![
                &*rep,
                drawAtPoint: objc2_foundation::NSPoint::new(pad as f64, pad as f64)
            ];
            let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
            let properties: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
            let png: Retained<NSData> = msg_send![
                &*bitmap,
                representationUsingType: 4usize,
                properties: &*properties
            ];
            png.to_vec()
        }
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    fn bitmap(text: bool) -> Vec<u8> {
        let (width, height) = (640usize, 160usize);
        let pixel_bytes = width * height * 3;
        let mut bytes = vec![255; 54 + pixel_bytes];
        bytes[..54].fill(0);
        bytes[..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
        bytes[34..38].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
        if text {
            use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
            use objc2_foundation::{ns_string, NSData, NSPoint, NSString};
            unsafe {
                let data = NSData::with_bytes(&bytes);
                let bitmap: Retained<AnyObject> =
                    msg_send![class!(NSBitmapImageRep), imageRepWithData: &*data];
                let context: Retained<AnyObject> = msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: &*bitmap];
                let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
                let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: &*context];
                let attributes: Retained<AnyObject> =
                    msg_send![class!(NSMutableDictionary), dictionary];
                let font: Retained<AnyObject> =
                    msg_send![class!(NSFont), boldSystemFontOfSize: 48f64];
                let color: Retained<AnyObject> = msg_send![class!(NSColor), blackColor];
                let _: () =
                    msg_send![&*attributes, setObject: &*font, forKey: ns_string!("NSFont")];
                let _: () =
                    msg_send![&*attributes, setObject: &*color, forKey: ns_string!("NSColor")];
                let label = NSString::from_str("HELLO NAMI");
                let _: () = msg_send![&*label, drawAtPoint: NSPoint::new(48.0, 48.0), withAttributes: &*attributes];
                let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
                let properties: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
                let png: Retained<NSData> =
                    msg_send![&*bitmap, representationUsingType: 4usize, properties: &*properties];
                return png.to_vec();
            }
        }
        bytes
    }
}
