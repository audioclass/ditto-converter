// Browser layout assertions against the actual app renderer and stylesheet.
// Run only with layout-smoke.html; no desktop IPC or media-success claims.
export function measureLayout() {
  const box = element => {
    const r = element.getBoundingClientRect();
    return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width, height: r.height };
  };
  const main = document.querySelector('main');
  const workspace = document.querySelector('.workspace');
  const queue = document.querySelector('.queue-body');
  const footer = document.querySelector('.export-bar');
  const inspector = document.querySelector('.inspector');
  const banner = document.querySelector('.error-banner');
  const q = box(queue), f = box(footer), w = box(workspace), m = box(main);
  const scrollBefore = queue.scrollTop;
  queue.scrollTop = queue.scrollHeight;
  const queueScroll = queue.scrollTop;
  queue.scrollTop = scrollBefore;
  const footerHits = [];
  for (let y = f.top + 5; y < Math.min(innerHeight, f.bottom); y += 8) {
    for (let x = 40; x < Math.min(w.right, innerWidth); x += 24) {
      if (document.elementFromPoint(x, y)?.closest('.queue-row')) footerHits.push({ x, y });
    }
  }
  const rows = document.querySelectorAll('.queue-row').length;
  const checks = {
    workspaceContainedInMain: w.top >= m.top - 1 && w.bottom <= m.bottom + 1,
    queueContainedInWorkspace: q.top >= w.top - 1 && q.bottom <= w.bottom + 1,
    queueDoesNotOverlapFooter: q.bottom <= f.top + 1 && footerHits.length === 0,
    footerFullyVisible: f.top >= 0 && f.bottom <= innerHeight + 1 && f.right <= innerWidth + 1,
    noDocumentOverflow: document.documentElement.scrollHeight <= innerHeight + 1 && document.documentElement.scrollWidth <= innerWidth + 1,
  };
  if (rows) {
    const row = document.querySelector('.queue-row');
    // The column heading scrolls with the queue, not as a sticky header. A
    // complete row must fit after scrolling it into view; do not reserve the
    // heading again (that would incorrectly reject the ordinary 600px layout).
    checks.queueHasUsableViewport = q.height >= Math.min(box(row).height, 78);
  }
  if (rows >= 11) checks.queueScrollsIndependently = queueScroll > 0;
  const inspectorBefore = inspector.scrollTop;
  inspector.scrollTop = inspector.scrollHeight;
  const inspectorScroll = inspector.scrollTop;
  inspector.scrollTop = inspectorBefore;
  checks.inspectorScrollDoesNotMoveFooter = Math.abs(box(footer).top - f.top) < 1;
  if (inspector.scrollHeight > inspector.clientHeight + 1) checks.inspectorScrollsIndependently = inspectorScroll > 0;
  for (const id of ['choose-folder', 'convert']) {
    const button = document.getElementById(id), r = box(button);
    checks[`${id}HitTargetVisible`] = r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth
      && [[r.left + 4, r.top + 4], [r.right - 4, r.bottom - 4], [(r.left + r.right) / 2, (r.top + r.bottom) / 2]]
        .every(([x, y]) => document.elementFromPoint(x, y)?.closest('button') === button);
  }
  return { viewport: { width: innerWidth, height: innerHeight }, rows, checks, main: m, workspace: w, queue: q, footer: f, inspector: box(inspector), banner: box(banner), queueScroll, footerHits };
}

export function assertLayout(result) {
  const failed = Object.entries(result.checks).filter(([, pass]) => !pass).map(([name]) => name);
  if (failed.length) throw new Error(`Layout acceptance failed: ${failed.join(', ')}`);
}
