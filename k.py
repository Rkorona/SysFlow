#!/usr/bin/env python3
import os, sys, argparse
from collections import Counter

EXTS = {'.kt', '.kts', '.sh', '.rs', '.md', '.ts', '.tsx',
        '.toml', '.json', '.css', '.pro', '.prop'}
SKIP_DIRS = {'.git', '.idea', '.gradle', '.vscode', 'node_modules',
             'build', 'dist', 'target', 'out', '.venv', 'venv', '__pycache__'}

def walk(root):
    for dp, dns, fns in os.walk(root, followlinks=True):
        dns[:] = sorted(d for d in dns if d not in SKIP_DIRS)
        for n in sorted(fns):
            yield os.path.abspath(os.path.join(dp, n))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('root', nargs='?', default='.')
    ap.add_argument('out', nargs='?', default='all_sources.md')
    ap.add_argument('--all', action='store_true')
    ap.add_argument('--max-mb', type=float, default=5.0)
    ap.add_argument('--diff', action='store_true', help='导出后做差集校验')
    args = ap.parse_args()

    root = args.root
    out_abs = os.path.abspath(args.out)
    included, skipped, big = [], [], []

    with open(args.out, 'w', encoding='utf-8') as f:
        f.write(f'# 已跳过目录: {", ".join(sorted(SKIP_DIRS))}\n\n')
        for p in walk(root):
            if p == out_abs:
                continue
            ext = os.path.splitext(p)[1].lower()
            rel = os.path.relpath(p, root).replace(os.sep, '/')
            if not args.all and ext not in EXTS:
                skipped.append((rel, 'ext')); continue

            size_mb = os.path.getsize(p) / 1048576
            f.write(rel + '\n')
            included.append(p)
            if size_mb > args.max_mb:
                f.write(f'[跳过内容：{size_mb:.1f}MB]\n\n')
                big.append((rel, size_mb)); continue
            try:
                with open(p, encoding='utf-8') as fh:
                    c = fh.read()
            except UnicodeDecodeError:
                skipped.append((rel, 'binary')); continue
            f.write(c if c.endswith('\n') else c + '\n')
            f.write('\n')

    print('\n=== 导出报告 ===')
    print(f'收录: {len(included)}  跳过: {len(skipped)}  超大: {len(big)}')
    for e, n in Counter(os.path.splitext(p)[1].lower() or '(none)' for p in included).most_common():
        print(f'  {e or "(无后缀)":8} {n}')
    if skipped:
        print(f'\n跳过明细 (前 50 条):')
        for rel, why in skipped[:50]:
            print(f'  [{why}] {rel}')
    print(f'\n校验命令: python {sys.argv[0]} {root} {args.out} --diff')

    if args.diff:
        included_abs = set(included)
        all_abs = set(walk(root))
        missing = all_abs - included_abs - {out_abs}
        print('\n=== 差集校验 ===')
        print(f'目录总文件(应用SKIP_DIRS后): {len(all_abs)}')
        print(f'已收录: {len(included_abs)}')
        print(f'未收录: {len(missing)}')
        for p in sorted(missing):
            rel = os.path.relpath(p, root).replace(os.sep, '/')
            print(f'  {rel}')
        if not missing:
            print('  ✅ 无遗漏')

if __name__ == '__main__':
    main()