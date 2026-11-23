export function RecordFeedback({ message }: { message: string | null }) {
  return <div className="record-feedback-live" aria-live="polite" aria-atomic="true">
    {message ? <div className="record-feedback" role="status">{message}</div> : null}
  </div>;
}
