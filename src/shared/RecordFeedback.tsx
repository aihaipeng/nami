export function RecordFeedback({ message }: { message: string | null }) {
  return message ? <div className="record-feedback" role="status">{message}</div> : null;
}
