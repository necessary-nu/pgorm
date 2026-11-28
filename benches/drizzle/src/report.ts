// A report's tables in Markdown: each workload's sides with the medians of
// their rounds, the ratios to the raw drivers' prepared paths, and the
// attribution's per-call, per-row, per-value and per-byte costs.

import type { Summary } from "./measure.ts";

interface Reported {
  readonly label: string;
  readonly started: string;
  readonly options: { readonly samples: number; readonly warmup: number; readonly rounds: number; readonly rows: number; readonly workers: number; readonly poolSize: number; readonly cursorBatch: number };
  readonly machine: { readonly node: string; readonly cpu: string | null; readonly os: string; readonly git_head: string | null };
  readonly server?: { readonly version: string };
  readonly rounds: readonly { readonly round: number; readonly start: { readonly loadavg: readonly number[]; readonly time_machine_running: boolean | null }; readonly end?: { readonly loadavg: readonly number[]; readonly time_machine_running: boolean | null } }[];
  readonly sides: readonly { readonly name: string }[];
  readonly absent: readonly { readonly side: string; readonly workload: string; readonly reason: string }[];
  readonly prepared: readonly { readonly side: string; readonly round: number; readonly statements: readonly { readonly name: string; readonly statement: string; readonly generic_plans: number; readonly custom_plans: number }[] }[];
  readonly measurements: readonly Summary[];
  readonly attribution: readonly Summary[];
}

const BASELINES = ["pg/prepared", "postgres/prepared"] as const;

function median(values: readonly number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? (sorted[middle] as number) : ((sorted[middle - 1] as number) + (sorted[middle] as number)) / 2;
}

const us = (ns: number) => (ns / 1000).toFixed(1);

function grouped(summaries: readonly Summary[]): Map<string, Summary[]> {
  const groups = new Map<string, Summary[]>();
  for (const summary of summaries) {
    const key = `${summary.side} ${summary.workload}`;
    groups.set(key, [...(groups.get(key) ?? []), summary]);
  }
  return groups;
}

function p50(groups: Map<string, Summary[]>, side: string, workload: string): number | undefined {
  const rounds = groups.get(`${side} ${workload}`);
  return rounds === undefined ? undefined : median(rounds.map((round) => round.p50_ns));
}

function workloadTables(report: Reported): string {
  const groups = grouped(report.measurements);
  const workloads = [...new Set(report.measurements.map((summary) => summary.workload))];
  const out: string[] = [];
  for (const workload of workloads) {
    out.push(`### ${workload}`, "");
    out.push("| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |");
    out.push("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for (const { name } of report.sides) {
      const rounds = groups.get(`${name} ${workload}`);
      if (rounds === undefined) {
        const reason = report.absent.find((entry) => entry.side === name && entry.workload === workload)?.reason;
        if (reason !== undefined) out.push(`| ${name} | n/a | | | | | | | | |`);
        continue;
      }
      const middle = median(rounds.map((round) => round.p50_ns));
      const ratios = BASELINES.map((baseline) => {
        const floor = p50(groups, baseline, workload);
        return floor === undefined ? "" : (middle / floor).toFixed(2);
      });
      const p50s = rounds.map((round) => round.p50_ns);
      out.push(
        `| ${name} | ${us(middle)} | ${us(median(rounds.map((round) => round.p95_ns)))} | ${us(median(rounds.map((round) => round.p99_ns)))} | ${us(Math.min(...rounds.map((round) => round.min_ns)))} | ${us(Math.max(...rounds.map((round) => round.max_ns)))} | ${us(Math.min(...p50s))}–${us(Math.max(...p50s))} | ${median(rounds.map((round) => round.operations_per_second)).toFixed(0)} | ${ratios[0]} | ${ratios[1]} |`,
      );
    }
    out.push("");
  }
  return out.join("\n");
}

function absences(report: Reported): string {
  if (report.absent.length === 0) return "";
  const out = ["### Not applicable", "", "| Side | Workload | Why |", "| --- | --- | --- |"];
  for (const entry of report.absent) out.push(`| ${entry.side} | ${entry.workload} | ${entry.reason} |`);
  return `${out.join("\n")}\n`;
}

function counts(workloads: readonly string[], base: string): number[] {
  return workloads
    .map((workload) => new RegExp(`^${base}@(\\d+)$`).exec(workload))
    .filter((match): match is RegExpExecArray => match !== null)
    .map((match) => Number(match[1]))
    .sort((a, b) => a - b);
}

function attributionTables(report: Reported): string {
  if (report.attribution.length === 0) return "";
  const groups = grouped(report.attribution);
  const workloads = [...new Set(report.attribution.map((summary) => summary.workload))];
  const sides = [...new Set(report.attribution.map((summary) => summary.side))];
  const out: string[] = ["### Rows: per call and per row", ""];
  out.push("Per row is the slope between the smallest and largest row counts, (p50 at the largest − p50 at the smallest) ÷ the rows between them; per value divides it by the columns.", "");
  out.push("| Side | Workload | " + "p50 µs at each count | per row µs | per value µs |");
  out.push("| --- | --- | --- | ---: | ---: |");
  const bases = [...new Set(workloads.filter((workload) => /@\d+$/.test(workload)).map((workload) => workload.replace(/@\d+$/, "")))].filter((base) => !base.startsWith("column_") && !base.startsWith("text_"));
  const columns: Record<string, number> = { rows_all: 3, rows_stream: 3, wide_all: 7, native_rows_all: 3, native_rows_stream: 3, native_wide_all: 7, row_objects: 3, loop_objects: 3, wide_row_objects: 7, wide_loop_objects: 7 };
  for (const base of bases) {
    for (const side of sides) {
      const at = counts(workloads, base).filter((count) => groups.has(`${side} ${base}@${count}`));
      if (at.length === 0) continue;
      const values = at.map((count) => p50(groups, side, `${base}@${count}`) as number);
      const first = at[0] as number;
      const last = at[at.length - 1] as number;
      const perRow = at.length > 1 ? ((values[values.length - 1] as number) - (values[0] as number)) / (last - first) : NaN;
      const width = columns[base] ?? NaN;
      out.push(
        `| ${side} | ${base} | ${at.map((count, index) => `${count}: ${us(values[index] as number)}`).join(", ")} | ${Number.isNaN(perRow) ? "" : (perRow / 1000).toFixed(3)} | ${Number.isNaN(perRow / width) ? "" : (perRow / width / 1000).toFixed(3)} |`,
      );
    }
  }
  out.push("");
  const poolWorkloads = workloads.filter((workload) => workload.startsWith("pool_query") || workload.startsWith("concurrent_pool"));
  if (poolWorkloads.length > 0) {
    out.push("### Pools", "", "Each side's pool-level query with one operation in flight (`@1worker`), and the pool configurations a side offers beside its default.", "");
    out.push("| Side | Workload | p50 µs | p95 µs | ops/s |", "| --- | --- | ---: | ---: | ---: |");
    for (const workload of poolWorkloads) {
      for (const side of sides) {
        const rounds = groups.get(`${side} ${workload}`);
        if (rounds === undefined) continue;
        out.push(`| ${side} | ${workload} | ${us(median(rounds.map((round) => round.p50_ns)))} | ${us(median(rounds.map((round) => round.p95_ns)))} | ${median(rounds.map((round) => round.operations_per_second)).toFixed(0)} |`);
      }
    }
    out.push("");
  }
  const columnWorkloads = workloads.filter((workload) => workload.startsWith("column_"));
  if (columnWorkloads.length > 0) {
    out.push("### One column of each type", "", "p50 of reading one column of bench_wide, and the same per row.", "");
    out.push(`| Column | ${sides.map((side) => `${side} p50 µs | per row ns`).join(" | ")} |`);
    out.push(`| --- | ${sides.map(() => "---: | ---:").join(" | ")} |`);
    for (const workload of columnWorkloads) {
      const rows = Number(/@(\d+)$/.exec(workload)?.[1]);
      const cells = sides.map((side) => {
        const value = p50(groups, side, workload);
        return value === undefined ? " | " : `${us(value)} | ${(value / rows).toFixed(0)}`;
      });
      out.push(`| ${workload.replace(/^column_/, "").replace(/@\d+$/, "")} | ${cells.join(" | ")} |`);
    }
    out.push("");
  }
  const textWorkloads = workloads.filter((workload) => workload.startsWith("text_"));
  if (textWorkloads.length > 0) {
    out.push("### Text of growing width", "", "p50 of reading 256 rows of text of each width; per byte is the slope from the narrowest to the widest.", "");
    out.push(`| Width | ${sides.map((side) => `${side} p50 µs`).join(" | ")} |`);
    out.push(`| --- | ${sides.map(() => "---:").join(" | ")} |`);
    const widths = textWorkloads.map((workload) => Number(/^text_(\d+)B/.exec(workload)?.[1])).sort((a, b) => a - b);
    for (const width of widths) {
      out.push(`| ${width} B | ${sides.map((side) => { const value = p50(groups, side, `text_${width}B@256`); return value === undefined ? "" : us(value); }).join(" | ")} |`);
    }
    const narrow = widths[0] as number;
    const wide = widths[widths.length - 1] as number;
    out.push(
      `| per byte ns | ${sides
        .map((side) => {
          const a = p50(groups, side, `text_${narrow}B@256`);
          const b = p50(groups, side, `text_${wide}B@256`);
          return a === undefined || b === undefined ? "" : ((b - a) / ((wide - narrow) * 256)).toFixed(3);
        })
        .join(" | ")} |`,
    );
    out.push("");
  }
  return out.join("\n");
}

function preparedTable(report: Reported): string {
  const first = report.prepared.filter((entry) => entry.round === 0);
  if (first.length === 0) return "";
  const out = ["### What each side's session holds prepared (round 0)", "", "| Side | Statements | Names, with generic/custom plans |", "| --- | ---: | --- |"];
  for (const entry of first) {
    out.push(`| ${entry.side} | ${entry.statements.length} | ${entry.statements.map((statement) => `\`${statement.name}\` (${statement.generic_plans}/${statement.custom_plans})`).join(", ")} |`);
  }
  return `${out.join("\n")}\n`;
}

function load(report: Reported): string {
  const out = ["### Load", "", "| Round | load at start (1/5/15 min) | at end | Time Machine running at start / end |", "| ---: | --- | --- | --- |"];
  for (const round of report.rounds) {
    const show = (loads: readonly number[] | undefined) => (loads === undefined ? "" : loads.map((value) => value.toFixed(2)).join(" / "));
    out.push(`| ${round.round} | ${show(round.start.loadavg)} | ${show(round.end?.loadavg)} | ${round.start.time_machine_running} / ${round.end?.time_machine_running} |`);
  }
  return `${out.join("\n")}\n`;
}

export function markdown(report: Reported): string {
  const { options } = report;
  return [
    `## ${report.label} run, ${report.started}`,
    "",
    `Node ${report.machine.node}, ${report.machine.cpu}, ${report.machine.os}; ${report.server?.version ?? "server unknown"}; HEAD ${report.machine.git_head}.`,
    `${options.warmup} warmup and ${options.samples} measured operations per workload, ${options.rounds} rounds, ${options.rows} rows, ${options.workers} workers over a pool of ${options.poolSize}, stream batches of ${options.cursorBatch} for pg-query-stream and postgres.js's cursor. Latencies are the medians of the rounds' percentiles.`,
    "",
    load(report),
    workloadTables(report),
    absences(report),
    preparedTable(report),
    attributionTables(report),
  ].join("\n");
}
