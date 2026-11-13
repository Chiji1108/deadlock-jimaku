"""PC の中の文字起こしで途中の字幕を作ったときの、進み方と負担を測る（DESIGN.md 9.10）。
アプリと同じやり方: 話している発話を、始まりから今までまとめて認識し直す。前の認識が終わってから 0.3 秒あけて次を頼む。
合成音声（data/tts/aya）の 1 発話ずつを、実時間で話しているものとして数える（実際には待たずに、時刻だけを進める）。

usage: python3 stt_live_eval_local.py SPEECH GLOSSARY.json MODEL [--heroes] [--cpu]
  MODEL は stt_terms_eval_local.py と同じ（speech.cpp の名前か、whisper.cpp の ggml のファイル）
出すもの: 1 回の認識の時間、文字が進む間隔、話している時間のうち認識していた割合、
  途中の文字が確定の文字とどれだけ違うか（確定の書き出しのうち、いちばん近いものとの文字誤り率）
"""

import io
import json
import sys
import wave
from pathlib import Path

import stt_terms_eval_local as L
from stt_eval_gemini import distance, normalize

HERE = Path(__file__).parent
INTERVAL = 0.3
MIN_SECONDS = 0.3


def cut(wav: Path, seconds: float) -> bytes:
    with wave.open(str(wav)) as w:
        frames = w.readframes(int(seconds * w.getframerate()))
        out = io.BytesIO()
        with wave.open(out, "wb") as o:
            o.setparams(w.getparams())
            o.writeframes(frames)
    return out.getvalue()


def main() -> None:
    args = sys.argv[1:]
    speech, glossary_path, model = [a for a in args if not a.startswith("--")]
    glossary = json.loads(Path(glossary_path).read_text())
    heroes = "、".join(sorted({e["names"]["ja"] for e in glossary["entities"].values() if e["kind"] == "hero" and e["names"].get("ja")}))
    prompt = heroes if "--heroes" in args else None
    process = L.serve(speech, model, "--cpu" in args)
    times, gaps, busy, spoken, off, partials, extra = [], [], 0.0, 0.0, [], 0, 0
    try:
        for wav in sorted((HERE / "data/tts/aya").glob("*.wav")):
            with wave.open(str(wav)) as w:
                length = w.getnframes() / w.getframerate()
            final = normalize(L.post(wav.read_bytes(), prompt)["text"])
            now, last, shown = MIN_SECONDS, 0.0, []
            while now < length:
                got = L.post(cut(wav, now), prompt)
                took = got["ms"] / 1000
                times.append(took)
                busy += took
                now += took
                gaps.append(now - last)
                last = now
                shown.append(normalize(got["text"]))
                now += INTERVAL
            spoken += length
            for text in shown:
                if not text:
                    continue
                partials += 1
                # 確定の書き出しのうち、いちばん近いものとの違い
                best = min(distance(final[:n], text) for n in range(len(final) + 1))
                off.append(best / len(text))
                # 確定より長く書いた（話していない続きを足した）
                extra += len(text) > len(final) + 2
    finally:
        process.terminate()
        process.wait()
    times.sort()
    gaps.sort()
    summary = {
        "model": Path(model).stem if model.endswith(".bin") else model,
        "prompt": "heroes" if prompt else "none",
        "cpu": "--cpu" in args,
        "requests": len(times),
        "medianMs": round(times[len(times) // 2] * 1000),
        "p90Ms": round(times[len(times) * 9 // 10] * 1000),
        "medianGapMs": round(gaps[len(gaps) // 2] * 1000),
        "busy": round(busy / spoken, 2),
        "partialErrorRate": round(sum(off) / len(off), 3),
        "longerThanFinal": extra,
        "partials": partials,
    }
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    path = HERE / "results/stt_live_local.json"
    results = json.loads(path.read_text()) if path.exists() else {}
    key = summary["model"] + ("+heroes" if prompt else "") + ("@cpu" if "--cpu" in args else "")
    results[key] = summary
    path.write_text(json.dumps(results, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
