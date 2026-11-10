# /// script
# requires-python = ">=3.12,<3.14"
# dependencies = [
#   "torch==2.14.0", "torchvision==0.29.0", "transformers==5.17.0", "safetensors==0.8.0",
#   "pillow==12.3.0", "huggingface-hub==1.31.0", "psutil",
# ]
# ///
"""PC の中で動く判定モデル（pplx-decider）を、Jev と同じ形の口で待たせる（DESIGN.md 9.7）。
split_eval の terms-jev に `local-…` のモデル名を渡すと、ここに聞きに来る。
モデルは、モデルと一緒に配られている開発元のコード（CHECKPOINT/source/src/pplx_decider）で動く。

usage: uv run local_decider_server.py CHECKPOINT DEVICE [each]
  CHECKPOINT は、ダウンロードしたモデルのフォルダ。DEVICE は mps（Apple シリコンの GPU）か cpu
  each を付けると、問いを 1 つずつに分けて聞く（このモデルの説明が勧める形）
"""

import json
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import psutil
import torch

PORT = 8765


def main() -> None:
    checkpoint, device = Path(sys.argv[1]), sys.argv[2]
    each = "each" in sys.argv[3:]
    sys.path.insert(0, str(checkpoint / "source" / "src"))
    from pplx_decider import Decider

    t0 = time.monotonic()
    decider = Decider.from_pretrained(checkpoint, device=device)
    print(f"loaded in {time.monotonic() - t0:.1f}s", flush=True)

    def memory() -> dict:
        gpu = torch.mps.driver_allocated_memory() if device == "mps" else 0
        return {"rssMb": round(psutil.Process().memory_info().rss / 1e6), "gpuMb": round(gpu / 1e6)}

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            state, questions = body["state"], body["questions"]
            if each:
                names = list(questions)
                parts = decider.decide([{"state": state, "questions": {n: questions[n]}} for n in names])
                answers = {n: part["answers"][n] for n, part in zip(names, parts, strict=True)}
                tokens = sum(part["usage"]["input_tokens"] for part in parts)
            else:
                res = decider.decide({"state": state, "questions": questions})
                answers, tokens = res["answers"], res["usage"]["input_tokens"]
            out = json.dumps({
                "answers": answers,
                "usage": {"input_tokens": tokens, "output_tokens": 0},
                "memory": memory(),
            }).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(out)))
            self.end_headers()
            self.wfile.write(out)

        def log_message(self, *_: object) -> None:
            pass

    print(f"listening on {PORT}", flush=True)
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()


if __name__ == "__main__":
    main()
