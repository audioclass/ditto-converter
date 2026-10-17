export function formatTime(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return '—';
  const ms = Math.round(Math.max(0, seconds) * 1000);
  return `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`;
}
export function validTrim(start: number, end: number | null, duration: number | null): boolean {
  return Number.isFinite(start) && start >= 0 && (end === null || (Number.isFinite(end) && end > start)) &&
    (duration === null || (start < duration && (end === null || end <= duration)));
}
export const activeStates = new Set(['acquiring', 'converting', 'validating', 'cancelling']);
export function selectionUpdates(jobs: { id: string; state: string; selected: boolean }[], selected: boolean) {
  return jobs.filter(j => !activeStates.has(j.state) && j.selected !== selected).map(j => ({ id: j.id, selected }));
}
export function queueView<T extends { position: number; selected: boolean; state: string }>(jobs: T[]) {
  return {
    ordered: [...jobs].sort((a, b) => a.position - b.position),
    selected: jobs.filter(j => j.selected).length,
    pending: jobs.filter(j => j.selected && ['queued', 'ready'].includes(j.state)).length,
    complete: jobs.filter(j => j.state === 'complete').length,
    active: jobs.some(j => activeStates.has(j.state)),
  };
}
export function parseTime(value: string): number | null {
  if (!/^\d+(?::\d{1,2}){0,2}(?:\.\d{1,3})?$/.test(value.trim())) return null;
  const parts = value.trim().split(':').map(Number);
  if (parts.slice(1).some(n => n >= 60)) return null;
  const seconds = parts.reduce((total, part) => total * 60 + part, 0);
  return Number.isFinite(seconds) ? seconds : null;
}
