# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""字幕のふりがなに、前の発話（文脈）を渡すと読みが良くなるかを比べる（DESIGN.md 6.4）。
文脈で読みが変わる短い発話を、文脈なし・ありで同じ回数ずつ聞き、狙いの語の読みが正しかった回数と、
答えが原文と合わず使えなかった回数（furigana::check に通らない）を数える。

usage: uv run furigana_context.py [REPEAT]
  出力: results/furigana_context.json
"""

import json
import os
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import httpx

MODEL = "gpt-6.1-terra"
OUT = Path(__file__).parent / "results" / "furigana_context.json"
# 文脈を付ける前の crates/core/src/furigana.rs の SYSTEM
SYSTEM = (
    "This is a Japanese subtitle of a streamer playing the video game Deadlock. "
    "Split it into words and give the hiragana reading of each word that contains kanji, reading it the way it is meant in context. "
    "A kanji compound is one segment with the reading of the whole word; never split a compound into single kanji. "
    "Keep okurigana, kana and punctuation in their own segments with ruby null. Joining the segments' text must give the sentence exactly."
)
# 今の crates/core/src/furigana.rs の SYSTEM と同じ
SYSTEM_CONTEXT = (
    "The input is JSON from a streamer playing the video game Deadlock: \"sentence\" is a Japanese subtitle, "
    "and \"context\" holds the previous subtitles (oldest first), given only to help you decide readings. "
    "Split the sentence (never the context) into words and give the hiragana reading of each word that contains kanji, reading it the way it is meant in context. "
    "A kanji compound is one segment with the reading of the whole word; never split a compound into single kanji. "
    "Keep okurigana, kana and punctuation in their own segments with ruby null. Joining the segments' text must give the sentence exactly."
)
SCHEMA = {
    "type": "object", "additionalProperties": False, "required": ["segments"],
    "properties": {"segments": {"type": "array", "items": {
        "type": "object", "additionalProperties": False, "required": ["text", "ruby"],
        "properties": {"text": {"type": "string"}, "ruby": {"type": ["string", "null"]}},
    }}},
}

# アプリと同じく前の発話は 3 つ。先頭は関係のない漢字混じりの雑談（区切りを崩さないかを見る）
CHATTER = "昨日の大会見た？"
# (前の発話, 字幕, 狙いの語, 正しい読み)。文だけでは読みが決まらない語と、文だけで決まる語（対照）
CASES = [
    (["ソウル全然足りないわ", "アイテム買えないじゃん"], "金ある？", "金", {"かね"}),
    (["ショップで何買う？", "ソウル6000ある"], "金使い切ろう", "金", {"かね"}),
    (["このレーン誰もいない", "敵全員ミッドに寄ってる"], "人気ないね", "人気", {"ひとけ"}),
    (["来月のアップデートいつだっけ", "たしか月初めって言ってた"], "一日からだっけ", "一日", {"ついたち"}),
    (["ずっと負けてる", "連敗止まらない"], "辛いわ", "辛", {"つら"}),
    (["敵が待ってるかも", "曲がるとき注意して"], "角に気をつけて", "角", {"かど"}),
    (["さっきのデス痛かった", "完全に読まれてた"], "相手が一枚上手だった", "上手", {"うわて"}),
    (["このアイテム効果すごい", "攻撃速度上がるやつ"], "大分変わるね", "大分", {"だいぶ"}),
    (["朝からずっと配信してる", "録画じゃないよ"], "生だからね", "生", {"なま"}),
    (["ガーディアン硬すぎる", "まだ壊せない"], "下手に突っ込むと死ぬ", "下手", {"へた"}),
]


def ask(http: httpx.Client, key: str, context: list[str] | None, sentence: str) -> list[dict]:
    user = sentence if context is None else json.dumps({"context": context, "sentence": sentence}, ensure_ascii=False)
    r = http.post(
        "https://api.openai.com/v1/chat/completions",
        headers={"Authorization": f"Bearer {key}"},
        json={
            "model": MODEL,
            "reasoning_effort": "none",
            "messages": [
                {"role": "system", "content": SYSTEM if context is None else SYSTEM_CONTEXT},
                {"role": "user", "content": user},
            ],
            "response_format": {"type": "json_schema", "json_schema": {"name": "furigana", "strict": True, "schema": SCHEMA}},
        },
        timeout=30,
    )
    r.raise_for_status()
    return json.loads(r.json()["choices"][0]["message"]["content"])["segments"]


def grade(segments: list[dict], sentence: str, word: str, expected: set[str]) -> str:
    """ok / wrong / rejected（原文と合わない）"""
    if "".join(s["text"] for s in segments) != sentence:
        return "rejected"
    for s in segments:
        if word in s["text"]:
            ruby = s["ruby"] or ""
            # 送り仮名や前後の語ごと答えることもあるので、正しい読みを含むかを見る（辛い → つらい、一枚上手 → いちまいうわて）
            return "ok" if any(e in ruby for e in expected) else "wrong"
    return "wrong"


def main() -> None:
    repeat = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    key = os.environ["OPENAI_API_KEY"]
    cases = [([CHATTER, *ctx], *rest) for ctx, *rest in CASES]
    jobs = [(i, mode) for i in range(len(CASES)) for mode in ("plain", "context") for _ in range(repeat)]
    with httpx.Client() as http, ThreadPoolExecutor(8) as pool:
        def run(job):
            i, mode = job
            ctx, sentence, word, expected = cases[i]
            segments = ask(http, key, ctx if mode == "context" else None, sentence)
            ruby = next((s["ruby"] for s in segments if word in s["text"]), None)
            return {"case": i, "mode": mode, "grade": grade(segments, sentence, word, expected), "ruby": ruby}
        results = list(pool.map(run, jobs))
    OUT.write_text(json.dumps(results, ensure_ascii=False, indent=1) + "\n")
    for i, (ctx, sentence, word, expected) in enumerate(CASES):
        row = []
        for mode in ("plain", "context"):
            rs = [r for r in results if r["case"] == i and r["mode"] == mode]
            ok = sum(r["grade"] == "ok" for r in rs)
            got = ",".join(sorted({str(r["ruby"]) if r["grade"] != "rejected" else "×" for r in rs}))
            row.append(f"{mode} {ok}/{len(rs)} ({got})")
        print(f"{sentence}  [{word}={'/'.join(expected)}]  " + "  ".join(row))
    for mode in ("plain", "context"):
        rs = [r for r in results if r["mode"] == mode]
        print(f"{mode}: ok {sum(r['grade'] == 'ok' for r in rs)}/{len(rs)}, rejected {sum(r['grade'] == 'rejected' for r in rs)}")


if __name__ == "__main__":
    main()
