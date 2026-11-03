# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""英語名を日本のプレイヤーが英語のまま読んだときのカタカナ表記を作る（DESIGN.md 8.4）。
音声認識は英語名で言われるとカタカナで書くので、公式の日本語名と違うものを別名の候補にする。

usage: uv run katakana_names.py GLOSSARY_JSON
  出力: results/aliases/katakana.output.json（aliases_research.py と同じ形）
"""

import json
import os
import sys
from pathlib import Path

import httpx

MODEL = "gpt-6.1-terra"
PER_REQUEST = 60
OUT = Path(__file__).parent / "results" / "aliases" / "katakana.output.json"
SYSTEM = (
    "These are terms from the video game Deadlock with their official English names. "
    "For each, write how a Japanese gamer would read the English name aloud, in katakana only "
    "(e.g. Extended Magazine → エクステンデッドマガジン, Infernus → インフェルナス, Cursed Relic → カースドレリック). "
    "Use the common Japanese katakana for English words and no middle dots. Answer every ref."
)
SCHEMA = {
    "type": "object", "additionalProperties": False, "required": ["terms"],
    "properties": {"terms": {"type": "array", "items": {
        "type": "object", "additionalProperties": False, "required": ["ref", "katakana"],
        "properties": {"ref": {"type": "string"}, "katakana": {"type": "string"}},
    }}},
}


def is_katakana(s: str) -> bool:
    return bool(s) and all("ァ" <= c <= "ヺ" or c == "ー" for c in s)


def main() -> None:
    entities = json.loads(Path(sys.argv[1]).read_text())["entities"]
    rows = [{"ref": r, "en": e["names"]["en"]} for r, e in entities.items() if e["names"].get("ja") and e["names"].get("en")]
    out = {}
    for i in range(0, len(rows), PER_REQUEST):
        chunk = rows[i : i + PER_REQUEST]
        r = httpx.post(
            "https://api.openai.com/v1/chat/completions",
            headers={"Authorization": f"Bearer {os.environ['OPENAI_API_KEY']}"},
            json={
                "model": MODEL, "reasoning_effort": "none",
                "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": json.dumps(chunk)}],
                "response_format": {"type": "json_schema", "json_schema": {"name": "katakana", "strict": True, "schema": SCHEMA}},
            },
            timeout=180,
        )
        r.raise_for_status()
        answers = {t["ref"]: t["katakana"] for t in json.loads(r.json()["choices"][0]["message"]["content"])["terms"]}
        for row in chunk:
            e = entities[row["ref"]]
            k = answers.get(row["ref"], "").replace("・", "")
            keep = is_katakana(k) and k != e["names"]["ja"]
            out[row["ref"]] = {"en": row["en"], "ja": e["names"]["ja"],
                               "aliases": [{"alias": k, "source": "katakana of the English name"}] if keep else []}
        print(f"{min(i + PER_REQUEST, len(rows))}/{len(rows)}")
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent=2))


main()
