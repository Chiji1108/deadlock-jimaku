import kawasakiIsumi from "../assets/streamers/kawasaki_isumi.webp?url";
import mikemorigin from "../assets/streamers/mikemorigin.webp?url";
import mmmMinami from "../assets/streamers/mmm_minami.webp?url";
import wawawawarabi from "../assets/streamers/wawawawarabi.webp?url";

/**
 * デドロ字幕を使っている配信者（載せる許可をもらった人だけ。足すときは、アイコンを `assets/streamers/` に置いて 1 行足す）。
 * `voice` は本人からもらったコメントで、言葉は変えない。
 * `chat` は、その人の配信のチャットに視聴者が書いたコメント（載せる許可をもらったものだけ。書いた人の名前は出さず、言葉は変えない）
 */
export const STREAMERS: { name: string; url: string; icon: string; voice: string; chat?: string[] }[] = [
  { name: "わっわらび", url: "https://www.twitch.tv/wawawawarabi", icon: wawawawarabi, voice: "キャラのアイコンが出てきて超かわいい！" },
  {
    name: "三毛森ギン",
    url: "https://www.twitch.tv/mikemorigin",
    icon: mikemorigin,
    voice: "今日コメントで精度高いって言われた！！！マジ凄い！神！日本語を拾う精度も高いみたい！！！サンキュー！",
  },
  { name: "川崎いすみ", url: "https://www.twitch.tv/kawasaki_isumi", icon: kawasakiIsumi, voice: "わーい！" },
  {
    name: "ミナミちゃんねる",
    url: "https://www.twitch.tv/mmm_minami",
    icon: mmmMinami,
    voice: "配信で字幕すごいねーって結構言われる",
    chat: ["英語字幕がデッドロック仕様なのすごいな", "字幕スキルアイコンとかキャラアイコン出るのめっちゃいい", "翻訳精度も高い"],
  },
];
