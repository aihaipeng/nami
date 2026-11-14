export function ActivityBeam({ label }: { label: string }) {
  return <div className="activity-beam" role="progressbar" aria-label={label}><span /></div>;
}
