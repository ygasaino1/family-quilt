#!/usr/bin/env python3
"""Check our Rust port of dot (crates/layout/src/dotrank.rs, dotorder.rs) against a real `dot`.

For every random graph (or file given with --file) it runs `dot -Tplain` and the example tool
`dotrank_cmp`, then compares the layer of every node and the left-to-right order inside every layer.
Use a `dot` built from the SAME Graphviz version the port is based on (see UPSTREAM); see README.md.

  cargo build -p family-layout --example dotrank_cmp
  python3 tools/graphviz-compare/compare.py --dot /path/to/dot --seeds 1-300
  python3 tools/graphviz-compare/compare.py --dot /path/to/dot --file graph.dot

Exit status 0 only if every graph agrees.
"""
import argparse, os, random, subprocess, sys, tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))


def gen(seed, path):
    """Three kinds of graph: genealogy-like (child->family->parent), sparse with cycles and repeats, layered DAG."""
    random.seed(seed)
    kind = seed % 3
    out = ['digraph g {', 'ranksep=1;', 'node [shape=point,width=0,height=0,label=""];', 'edge [style=invis];']
    if kind == 0:
        P = random.randint(10, 400)
        F = random.randint(3, P // 2)
        for f in range(F):
            for c in random.sample(range(P), random.randint(1, 5)):
                out.append(f'"p{c}"->"f{f}";')
            for p in random.sample(range(P), random.randint(1, 2)):
                out.append(f'"f{f}"->"p{p}";')
    elif kind == 1:
        N = random.randint(5, 300)
        for _ in range(random.randint(N // 2, N * 2)):
            out.append(f'"n{random.randrange(N)}"->"n{random.randrange(N)}";')
    else:
        N = random.randint(10, 300)
        for _ in range(random.randint(N, N * 2)):
            a, b = random.randrange(N), random.randrange(N)
            if a != b:
                out.append(f'"n{min(a, b)}"->"n{max(a, b)}";')
    out.append('}')
    with open(path, 'w') as f:
        f.write('\n'.join(out) + '\n')


def reference(dot, path):
    text = subprocess.run([dot, '-Tplain', path], capture_output=True, text=True, check=True).stdout
    pts = {}
    for line in text.splitlines():
        p = line.split()
        if p and p[0] == 'node':
            pts[p[1].strip('"')] = (float(p[3]), float(p[2]))  # (y, x)
    levels = sorted({round(y, 4) for y, _ in pts.values()}, reverse=True)  # rank 0 = largest y
    return {n: (levels.index(round(y, 4)), x) for n, (y, x) in pts.items()}


def ours(tool, path):
    text = subprocess.run([tool, path], capture_output=True, text=True, check=True).stdout
    return {l.split()[0]: (int(l.split()[1]), int(l.split()[2])) for l in text.splitlines()}


def check(dot, tool, path):
    ref, got = reference(dot, path), ours(tool, path)
    if set(ref) != set(got):
        return 'different node sets'
    # layers: rank by rank; our rank 0 is dot's rank 0 (the top), so compare the layer numbers directly
    bad_rank = [n for n in ref if ref[n][0] != got[n][0]]
    if bad_rank:
        return f'{len(bad_rank)} nodes on a different layer, e.g. {bad_rank[:3]}'

    def order(pairs):
        by = {}
        for n, (r, x) in pairs.items():
            by.setdefault(r, []).append((x, n))
        return {r: [n for _, n in sorted(v)] for r, v in by.items()}

    a, b = order(ref), order(got)
    bad = [r for r in a if a[r] != b[r]]
    return f'order differs in layers {bad[:8]}' if bad else None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--dot', required=True, help='a dot built from the same Graphviz version as the port')
    ap.add_argument('--tool', default=os.path.join(ROOT, 'target', 'debug', 'examples', 'dotrank_cmp'))
    ap.add_argument('--seeds', default='1-100', help='range of random graphs, e.g. 1-300')
    ap.add_argument('--file', help='check this one .dot file instead')
    args = ap.parse_args()
    if not os.path.exists(args.tool):
        sys.exit(f'{args.tool} not found: run  cargo build -p family-layout --example dotrank_cmp')
    bad = 0
    if args.file:
        r = check(args.dot, args.tool, args.file)
        print(args.file, 'agrees' if r is None else f'DIFFERS: {r}')
        sys.exit(0 if r is None else 1)
    lo, hi = (int(x) for x in args.seeds.split('-'))
    with tempfile.TemporaryDirectory() as tmp:
        for seed in range(lo, hi + 1):
            path = os.path.join(tmp, f'r{seed}.dot')
            gen(seed, path)
            r = check(args.dot, args.tool, path)
            if r is not None:
                bad += 1
                print(f'seed {seed}: {r}')
    print(f'{hi - lo + 1 - bad} of {hi - lo + 1} graphs agree')
    sys.exit(0 if bad == 0 else 1)


if __name__ == '__main__':
    main()
