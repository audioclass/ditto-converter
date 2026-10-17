export interface Job {
  id: string; title: string;
  source: { kind: 'local'; path: string } | { kind: 'online'; url: string; provider: string };
  position: number; duration: number | null; trim_start: number; trim_end: number | null;
  selected: boolean; state: string; progress: number | null; error: string | null; output: string | null;
}
export interface Snapshot { jobs: Job[]; output_dir: string | null; running: boolean; online_available: boolean; component_status: string; notice?: string | null; }
export interface OnlineEntry { title: string; url: string; provider: string; duration: number | null; position: number; available: boolean; }
