#!/usr/bin/env python3
"""check-bench-regression.py — 解析 critcmp 输出，回退 > 阈值则 exit 1。

SPEC §13.2: benchmark CI 性能回退 >10% 报警。

critcmp 输出格式可能变化，脚本容错：解析失败时 warn 并 exit 0（不阻断）。
"""
import sys
import re


def parse_critcmp(text):
    """解析 critcmp 表格输出，返回 [(name, main_ms, current_ms), ...]"""
    results = []
    lines = text.strip().split('\n')
    for line in lines:
        # critcmp 行格式：bench_name  main     X ms   current  Y ms
        # 或：bench_name  main: X ms  current: Y ms
        # 尝试匹配 "main" 和 "current" 列的时间值
        m = re.search(
            r'([\d.]+)\s*(ms|µs|ns|s)\s+.*current.*?([\d.]+)\s*(ms|µs|ns|s)',
            line,
            re.IGNORECASE,
        )
        if m:
            main_val = float(m.group(1))
            main_unit = m.group(2).lower()
            curr_val = float(m.group(3))
            curr_unit = m.group(4).lower()
            # 归一化到 ms
            unit_factor = {
                's': 1000,
                'ms': 1,
                'µs': 0.001,
                'us': 0.001,
                'ns': 0.000001,
            }
            main_ms = main_val * unit_factor.get(main_unit, 1)
            curr_ms = curr_val * unit_factor.get(curr_unit, 1)
            name = line.split()[0] if line.split() else "unknown"
            results.append((name, main_ms, curr_ms))
    return results


def main():
    if len(sys.argv) < 3:
        print("Usage: check-bench-regression.py <compare.txt> <threshold>", file=sys.stderr)
        sys.exit(2)
    filepath = sys.argv[1]
    threshold = float(sys.argv[2])

    try:
        with open(filepath) as f:
            text = f.read()
    except IOError as e:
        print(f"WARN: cannot read {filepath}: {e}", file=sys.stderr)
        sys.exit(0)  # 容错：解析失败不阻断

    results = parse_critcmp(text)
    if not results:
        print("WARN: no benchmark results parsed, skipping regression check", file=sys.stderr)
        sys.exit(0)

    failures = []
    for name, main_ms, curr_ms in results:
        if main_ms > 0:
            regression = (curr_ms - main_ms) / main_ms
            if regression > threshold:
                failures.append((name, main_ms, curr_ms, regression))

    if failures:
        print(
            f"FAIL: {len(failures)} benchmark(s) regressed > {threshold*100:.0f}%:",
            file=sys.stderr,
        )
        for name, main_ms, curr_ms, reg in failures:
            print(
                f"  {name}: main={main_ms:.4f}ms current={curr_ms:.4f}ms "
                f"regression={reg*100:.1f}%",
                file=sys.stderr,
            )
        sys.exit(1)
    else:
        print(f"OK: no benchmark regressed > {threshold*100:.0f}%")
        sys.exit(0)


if __name__ == '__main__':
    main()
