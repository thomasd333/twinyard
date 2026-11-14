/** Line diff (Myers O((N+M)·D)) aligned into side-by-side rows. */

export type RowKind = "same" | "del" | "add" | "chg";

export interface Row {
  kind: RowKind;
  /** 1-based line numbers; null = empty cell. */
  ln: number | null;
  rn: number | null;
  l: string;
  r: string;
}

type Op = { t: "=" | "-" | "+"; a: number; b: number };

function myers(a: string[], b: string[]): Op[] {
  const n = a.length,
    m = b.length,
    max = n + m;
  const v = new Int32Array(2 * max + 2);
  const trace: Int32Array[] = [];
  for (let d = 0; d <= max; d++) {
    trace.push(v.slice());
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[max + k - 1] < v[max + k + 1]) ? v[max + k + 1] : v[max + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) x++, y++;
      v[max + k] = x;
      if (x >= n && y >= m) return backtrack(trace, a, b, max, d);
    }
  }
  return [];
}

function backtrack(trace: Int32Array[], a: string[], b: string[], max: number, dEnd: number): Op[] {
  const ops: Op[] = [];
  let x = a.length,
    y = b.length;
  for (let d = dEnd; d > 0; d--) {
    const v = trace[d];
    const k = x - y;
    const prevK = k === -d || (k !== d && v[max + k - 1] < v[max + k + 1]) ? k + 1 : k - 1;
    const px = v[max + prevK],
      py = px - prevK;
    while (x > px && y > py) ops.push({ t: "=", a: --x, b: --y });
    if (x === px) ops.push({ t: "+", a: x, b: --y });
    else ops.push({ t: "-", a: --x, b: y });
  }
  while (x > 0 && y > 0) ops.push({ t: "=", a: --x, b: --y });
  return ops.reverse();
}

export function splitLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split(/\r\n|\n|\r/);
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** Side-by-side rows; neighbouring deletes+adds are paired as "chg". */
export function diffRows(left: string, right: string): Row[] {
  const a = splitLines(left),
    b = splitLines(right);
  // Common prefix/suffix outside Myers keeps big, mostly equal files fast.
  let pre = 0;
  while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++;
  let suf = 0;
  while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++;
  const mid = myers(a.slice(pre, a.length - suf), b.slice(pre, b.length - suf));

  const rows: Row[] = [];
  const same = (i: number, j: number) => rows.push({ kind: "same", ln: i + 1, rn: j + 1, l: a[i], r: b[j] });
  for (let i = 0; i < pre; i++) same(i, i);
  let dels: number[] = [],
    adds: number[] = [];
  const flush = () => {
    const n = Math.max(dels.length, adds.length);
    for (let k = 0; k < n; k++) {
      const i = dels[k],
        j = adds[k];
      rows.push({
        kind: i !== undefined && j !== undefined ? "chg" : i !== undefined ? "del" : "add",
        ln: i !== undefined ? i + 1 : null,
        rn: j !== undefined ? j + 1 : null,
        l: i !== undefined ? a[i] : "",
        r: j !== undefined ? b[j] : "",
      });
    }
    dels = [];
    adds = [];
  };
  for (const op of mid) {
    if (op.t === "=") {
      flush();
      same(op.a + pre, op.b + pre);
    } else if (op.t === "-") dels.push(op.a + pre);
    else adds.push(op.b + pre);
  }
  flush();
  for (let k = suf; k > 0; k--) same(a.length - k, b.length - k);
  return rows;
}

/** Indexes of rows where a block of changes starts. */
export function hunks(rows: Row[]): number[] {
  const out: number[] = [];
  rows.forEach((r, i) => {
    if (r.kind !== "same" && (i === 0 || rows[i - 1].kind === "same")) out.push(i);
  });
  return out;
}
