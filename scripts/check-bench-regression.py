#!/usr/bin/env python3
"""check-bench-regression.py — 解析 critcmp 输出，回退 > 阈值则 exit 1。

SPEC §13.2: benchmark CI 性能回退 >10% 报警。

critcmp 表格输出形如（每数据行两列时间值，main 在前 current 在后）::

    benchmark                        main                                current
    ---------------------------------------------------------------------------
    hybrid_search_10k_topk10         1.23 ms         1.40 ms    +13.8% ± 0.5%

解析策略：每行抓取所有 ``<数值> <单位>`` token（单位支持 ms/µs/us/ns/s），
取前两个分别作 main / current。解析失败时 warn 并 exit 0（不阻断，SPEC 容错）。
"""
import sys
import re

# 时间 token：数值 + 单位。µ 用 unicode 字面，同时接受 ASCII us 兜底。
_TIME_RE = re.compile(r'([\d.]+)\s*(ms|µs|us|ns|s)\b', re.IGNORECASE)

_UNIT_FACTOR = {
    's': 1000.0,
    'ms': 1.0,
    'µs': 0.001,
    'us': 0.001,
    'ns': 0.000001,
}


def _to_ms(val: float, unit: str) -> float:
    return val * _UNIT_FACTOR.get(unit.lower(), 1.0)


def parse_critcmp(text):
    """解析 critcmp 表格输出，返回 [(name, main_ms, current_ms), ...]。

    每数据行取前两个 ``<数值 单位>`` token 作 main / current；
    不足两个的行（表头/分隔线）跳过。
    """
    results = []
    for line in text.strip().split('\n'):
        tokens = _TIME_RE.findall(line)
        if len(tokens) < 2:
            continue
        main_val = float(tokens[0][0])
        main_ms = _to_ms(main_val, tokens[0][1])
        curr_val = float(tokens[1][0])
        curr_ms = _to_ms(curr_val, tokens[1][1])
        name = line.split()[0] if line.split() else "unknown"
        results.append((name, main_ms, curr_ms))
    return results


def main():
    if len(sys.argv) < 3:
        print("Usage: check-bench-regression.py <compare.txt> <threshold>",
              file=sys.stderr)
        sys.exit(2)
    filepath = sys.argv[1]
    threshold = float(sys.argv[2])

    try:
        with open(filepath) as f:
            text = f.read()
    except IOError as e:
        print(f"WARN: cannot read {filepath}: {e}", file=sys.stderr)
        sys.exit(0)  # 容错：读失败不阻断

    results = parse_critcmp(text)
    if not results:
        print("WARN: no benchmark results parsed, skipping regression check",
              file=sys.stderr)
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
