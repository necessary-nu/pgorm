import type { Options } from "./options.ts";

/** One workload's distribution in one round, its latencies in nanoseconds. */
export interface Summary {
  readonly side: string;
  readonly workload: string;
  readonly round: number;
  readonly samples: number;
  /**
   * Sequential workloads: samples over the sum of their latencies, so the
   * untimed result checks between operations do not count. Concurrent ones:
   * every worker's samples over the wall-clock time from the start gate to
   * the last worker's end, as driver_baseline reports it.
   */
  readonly operations_per_second: number;
  readonly elapsed_ns: number;
  readonly p50_ns: number;
  readonly p95_ns: number;
  readonly p99_ns: number;
  readonly min_ns: number;
  readonly max_ns: number;
  readonly sorted_latency_ns?: readonly number[];
}

/**
 * An operation, and the check every result it gives must pass. Methods, so
 * an operation of any result type is an `Operation`.
 */
export interface Operation<T = unknown> {
  run(): Promise<T>;
  check(result: T): void;
}

/** An operation whose check is typed by what it runs. */
export function op<T>(run: () => Promise<T>, check: (result: T) => void): Operation {
  return { run, check } as Operation<T> as Operation;
}

const collect = (globalThis as { gc?: () => void }).gc;

/**
 * Collect garbage between workloads when the process runs with
 * `--expose-gc`, so that one workload's garbage is not collected inside the
 * next one's timed operations.
 */
export function settle(): void {
  collect?.();
}

export function gcExposed(): boolean {
  return collect !== undefined;
}

/** The nearest-rank quantile driver_baseline takes, of an ascending array. */
function quantile(sorted: Float64Array, percent: number): number {
  const count = sorted.length;
  return sorted[Math.min(Math.ceil((count * percent) / 100) - 1, count - 1)] as number;
}

export function summarize(
  side: string,
  workload: string,
  round: number,
  times: Float64Array,
  elapsed: number,
  operationsPerSecond: number,
  rawSamples: boolean,
): Summary {
  const sorted = times.slice().sort();
  return {
    side,
    workload,
    round,
    samples: sorted.length,
    operations_per_second: operationsPerSecond,
    elapsed_ns: elapsed,
    p50_ns: quantile(sorted, 50),
    p95_ns: quantile(sorted, 95),
    p99_ns: quantile(sorted, 99),
    min_ns: sorted[0] as number,
    max_ns: sorted[sorted.length - 1] as number,
    ...(rawSamples ? { sorted_latency_ns: Array.from(sorted) } : {}),
  };
}

function now(): bigint {
  return process.hrtime.bigint();
}

/**
 * Time `operation` `options.samples` times after `options.warmup` untimed
 * runs, checking every result, the warmup's included, so an error or a
 * wrong answer never counts as a fast operation. The check runs after the
 * clock stops.
 */
export async function measure<T>(
  side: string,
  workload: string,
  round: number,
  options: Options,
  operation: Operation<T>,
): Promise<Summary> {
  for (let at = 0; at < options.warmup; at++) operation.check(await operation.run());
  const times = new Float64Array(options.samples);
  const started = now();
  let busy = 0;
  for (let at = 0; at < options.samples; at++) {
    const start = now();
    const result = await operation.run();
    const took = Number(now() - start);
    times[at] = took;
    busy += took;
    operation.check(result);
  }
  const elapsed = Number(now() - started);
  const summary = summarize(side, workload, round, times, elapsed, options.samples / (busy / 1e9), options.rawSamples);
  report(summary);
  return summary;
}

export async function measureConcurrent<T>(
  side: string,
  workload: string,
  round: number,
  options: Options,
  operation: Operation<T>,
): Promise<Summary> {
  const warm = Array.from({ length: options.workers }, () => Promise.withResolvers<void>());
  const gate = Promise.withResolvers<void>();
  const workers = warm.map(async (ready) => {
    for (let at = 0; at < options.warmup; at++) operation.check(await operation.run());
    ready.resolve();
    await gate.promise;
    const times = new Float64Array(options.samples);
    for (let at = 0; at < options.samples; at++) {
      const start = now();
      const result = await operation.run();
      times[at] = Number(now() - start);
      operation.check(result);
    }
    return times;
  });
  // A worker that fails during warmup would otherwise leave the gate shut;
  // none can finish before the gate opens.
  await Promise.race([Promise.all(warm.map((ready) => ready.promise)), Promise.all(workers)]);
  const started = now();
  gate.resolve();
  const results = await Promise.all(workers);
  const elapsed = Number(now() - started);
  const times = new Float64Array(options.samples * options.workers);
  results.forEach((worker, index) => times.set(worker, index * options.samples));
  const summary = summarize(side, workload, round, times, elapsed, times.length / (elapsed / 1e9), options.rawSamples);
  report(summary);
  return summary;
}

function report(summary: Summary): void {
  const micros = (ns: number) => (ns / 1000).toFixed(1);
  console.error(
    `round ${summary.round}: ${summary.side} ${summary.workload}: p50=${micros(summary.p50_ns)}us p95=${micros(summary.p95_ns)}us ops/s=${summary.operations_per_second.toFixed(0)}`,
  );
}
