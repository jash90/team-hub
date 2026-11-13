export function ProgressBar({ done, inProgress = 0, total }: { done: number; inProgress?: number; total: number }) {
  const share = (value: number) => (total > 0 ? `${(value / total) * 100}%` : '0%');

  return (
    <div className="progress" role="img" aria-label={`${done} / ${total}`}>
      <span className="progress-done" style={{ width: share(done) }} />
      <span className="progress-doing" style={{ width: share(inProgress) }} />
    </div>
  );
}
