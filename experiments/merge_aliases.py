"""調べた別名の候補をまとめて data/aliases.ja.json を作る（DESIGN.md 8.4）。

入力は results/aliases/*.output.json（aliases_research.py やサブエージェントの調査、katakana_names.py の出力。
どれも { ref: { en, ja, aliases: [{ alias, source }] } }）と、今の data/aliases.ja.json（手で足したもの）。

usage: python3 merge_aliases.py
"""

import json
from pathlib import Path

ROOT = Path(__file__).parent.parent
OUT = ROOT / "data" / "aliases.ja.json"
CANDIDATES = Path(__file__).parent / "results" / "aliases"

# 出典はあるが誤記とみられるもの（音声認識もこうは書かない）
TYPOS = {
    "モンスタラウンド",
    "アルカミカルファイア",
    "スピリットレジエンス",
    "エスカレートレジエンス",
    "ヴェルネラビリティ",
}
# 英語名のカタカナ読みは、短いとふつうの言葉と重なる（エヴァ、コンボ、レイク）
MIN_KATAKANA_READING = 5


def main() -> None:
    merged: dict[str, list[str]] = {
        ref: list(aliases) for ref, aliases in json.loads(OUT.read_text()).items()
    }
    names: dict[str, str] = {}
    for path in sorted(CANDIDATES.glob("*.output.json")):
        for ref, entry in json.loads(path.read_text()).items():
            names[ref] = entry["ja"]
            for a in entry["aliases"]:
                alias = a["alias"].strip()
                reading = a["source"] == "katakana of the English name"
                if (
                    len(alias) < 2
                    or alias.isascii()  # 英語の旧名は日本語の音声認識ではまず出てこない
                    or alias in TYPOS
                    or (reading and len(alias) < MIN_KATAKANA_READING)
                ):
                    continue
                merged.setdefault(ref, []).append(alias)

    official = set(names.values())
    out: dict[str, list[str]] = {}
    for ref, aliases in sorted(merged.items()):
        seen: set[str] = set()
        keep = []
        for alias in aliases:
            # どれかの公式名そのものは要らない（照合では公式名が優先する）。
            # 中黒を抜いただけの表記は残す（音声認識は中黒を書かない）
            if alias in official or alias in seen:
                continue
            seen.add(alias)
            keep.append(alias)
        if keep:
            out[ref] = keep
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n")
    print(f"{len(out)} terms, {sum(len(v) for v in out.values())} aliases -> {OUT.relative_to(ROOT)}")


main()
