#!/usr/bin/env python3
import argparse
import csv
from collections import defaultdict

try:
    import matplotlib.pyplot as plt
except ModuleNotFoundError:
    plt = None


VARIANTS = [
    ("paper_original", "original", "tab:blue"),
    ("auto_best", "generated best", "tab:green"),
    ("paper_manual", "manual rewrite", "tab:orange"),
]


def read_rows(path):
    with open(path, newline="") as handle:
        rows = list(csv.DictReader(handle))

    grouped = defaultdict(dict)
    for row in rows:
        grouped[row["id"]][row["variant"]] = row

    return grouped


def plot_grouped_bars(grouped, output, metric):
    if plt is None:
        return plot_grouped_bars_svg(grouped, output, metric)

    ids = list(grouped)
    x = list(range(len(ids)))
    width = 0.24

    plt.rcParams.update({
        "font.size": 18,
        "axes.facecolor": "whitesmoke",
        "font.family": "serif",
        "figure.figsize": (14, 7),
    })

    fig, ax = plt.subplots()
    ax.grid(color="white", linestyle="-", linewidth=3, zorder=1)

    present_variants = [variant for variant in VARIANTS if any(variant[0] in grouped[id_] for id_ in ids)]
    if len(present_variants) == 1:
        offsets = [0.0]
    else:
        start = -width * (len(present_variants) - 1) / 2
        step = width
        offsets = [start + step * idx for idx in range(len(present_variants))]

    for offset, (variant, label, color) in zip(offsets, present_variants):
        values = []
        positions = []
        for idx, id_ in enumerate(ids):
            row = grouped[id_].get(variant)
            if row is None:
                continue
            values.append(value_for(row, metric))
            positions.append(x[idx] + offset)

        ax.bar(positions, values, width=width, label=label, color=color, zorder=4)

    ax.set_xticks(x)
    ax.set_xticklabels(ids)
    ax.set_ylabel(label_for(metric))
    ax.legend(loc="upper center", bbox_to_anchor=(0.5, 1.18), ncol=len(present_variants))
    fig.tight_layout()
    fig.savefig(output, bbox_inches="tight")
    return output


def plot_grouped_bars_svg(grouped, output, metric):
    ids = list(grouped)
    present_variants = [variant for variant in VARIANTS if any(variant[0] in grouped[id_] for id_ in ids)]
    values = {
        (id_, variant): value_for(row, metric)
        for id_, variants in grouped.items()
        for variant, row in variants.items()
    }
    max_value = max(values.values(), default=1.0)

    width = 1200
    height = 620
    margin_left = 80
    margin_right = 30
    margin_top = 70
    margin_bottom = 90
    plot_width = width - margin_left - margin_right
    plot_height = height - margin_top - margin_bottom
    group_width = plot_width / max(len(ids), 1)
    bar_width = min(24, group_width / max(len(present_variants) + 1, 1))
    color_map = {variant: color for variant, _, color in VARIANTS}
    label_map = {variant: label for variant, label, _ in VARIANTS}
    svg_colors = {
        "tab:blue": "#1f77b4",
        "tab:green": "#2ca02c",
        "tab:orange": "#ff7f0e",
    }

    def sx(group_idx, variant_idx):
        base = margin_left + group_idx * group_width + group_width / 2
        offset = (variant_idx - (len(present_variants) - 1) / 2) * bar_width * 1.25
        return base + offset - bar_width / 2

    def sy(value):
        return margin_top + plot_height - (value / max_value) * plot_height

    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        f'<rect x="{margin_left}" y="{margin_top}" width="{plot_width}" height="{plot_height}" fill="whitesmoke"/>',
        '<style>text{font-family:serif;font-size:24px}.small{font-size:18px}</style>',
    ]

    for tick in range(6):
        value = max_value * tick / 5
        y = sy(value)
        lines.append(f'<line x1="{margin_left}" y1="{y:.2f}" x2="{width - margin_right}" y2="{y:.2f}" stroke="white" stroke-width="3"/>')
        lines.append(f'<text x="{margin_left - 12}" y="{y + 6:.2f}" text-anchor="end" class="small">{value:.2g}</text>')

    legend_x = margin_left
    for variant, label, color in present_variants:
        fill = svg_colors.get(color, color)
        lines.append(f'<rect x="{legend_x}" y="22" width="36" height="18" fill="{fill}"/>')
        lines.append(f'<text x="{legend_x + 46}" y="39">{label}</text>')
        legend_x += 260

    for group_idx, id_ in enumerate(ids):
        for variant_idx, (variant, _, color) in enumerate(present_variants):
            value = values.get((id_, variant))
            if value is None:
                continue
            x = sx(group_idx, variant_idx)
            y = sy(value)
            h = margin_top + plot_height - y
            fill = svg_colors.get(color_map[variant], color)
            lines.append(f'<rect x="{x:.2f}" y="{y:.2f}" width="{bar_width:.2f}" height="{h:.2f}" fill="{fill}"/>')

        x = margin_left + group_idx * group_width + group_width / 2
        lines.append(f'<text x="{x:.2f}" y="{height - 42}" text-anchor="middle">{id_}</text>')

    lines.append(f'<text x="{width / 2}" y="{height - 12}" text-anchor="middle" class="small">{label_for(metric)}</text>')
    lines.append("</svg>")

    if not output.endswith(".svg"):
        output = output.rsplit(".", 1)[0] + ".svg"
    with open(output, "w") as handle:
        handle.write("\n".join(lines))
    return output


def value_for(row, metric):
    if metric == "mean-ms":
        return float(row["exec_mean_ns"]) / 1_000_000.0
    if metric == "speedup":
        return float(row["speedup_vs_original"])
    raise ValueError(metric)


def label_for(metric):
    if metric == "mean-ms":
        return "mean execution time [ms]"
    if metric == "speedup":
        return "speedup vs original"
    raise ValueError(metric)


def main():
    parser = argparse.ArgumentParser(description="Plot grouped bars from rq-paper-rewrite-bench CSV.")
    parser.add_argument("csv", nargs="?", default="target/paper-rewrite-bench.csv")
    parser.add_argument("-o", "--output", default="target/paper-rewrite-bench.png")
    parser.add_argument("--metric", choices=["mean-ms", "speedup"], default="mean-ms")
    args = parser.parse_args()

    grouped = read_rows(args.csv)
    output = plot_grouped_bars(grouped, args.output, args.metric)
    print(f"Plot written to {output}")


if __name__ == "__main__":
    main()
