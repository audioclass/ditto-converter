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
  if (rows >= 11) checks.queueScrollsIndependently = queueScroll > 0;
  return { viewport: { width: innerWidth, height: innerHeight }, rows, checks, main: m, workspace: w, queue: q, footer: f, inspector: box(inspector), banner: box(banner), queueScroll, footerHits };
}

export function assertLayout(result) {
  const failed = Object.entries(result.checks).filter(([, pass]) => !pass).map(([name]) => name);
  if (failed.length) throw new Error(`Layout acceptance failed: ${failed.join(', ')}`);
}
