/** Wraps `run` so it runs at most once per `everyMs`, counting from when it's
 *  wrapped: calls in the first `everyMs` are dropped too. */
export function throttled(run: () => void, everyMs: number, now: () => number = Date.now): () => void {
  let last = now();
  return () => {
    if (now() - last < everyMs) return;
    last = now();
    run();
  };
}
