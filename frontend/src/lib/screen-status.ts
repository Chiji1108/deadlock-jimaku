// ゲームの画面の読み取りの状態の言い方（DESIGN.md 18.5、18.8）。読み取りの状態の欄と、OBS のソースの欄で同じ文を出す
import type { ScreenStatus } from "../generated/ScreenStatus";

/** OBS から絵をもらえない理由。OBS の話でなければ undefined */
export function obsProblem(status: ScreenStatus): string | undefined {
  switch (status.state) {
    case "noObs":
      return "OBS につながりません（OBS を開いて、WebSocket サーバーを有効にしてください）";
    case "obsPassword":
      return "OBS のパスワードが違うか、まだ入れていません";
    case "noInput":
      return "ゲームの画面のソースを選んでください（選んだソースが OBS にありません）";
    case "failed":
      return `読めません（${status.detail}）`;
  }
}
