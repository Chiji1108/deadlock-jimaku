import kawasakiIsumi from "../assets/streamers/kawasaki_isumi.webp?url";
import mikemorigin from "../assets/streamers/mikemorigin.webp?url";
import wawawawarabi from "../assets/streamers/wawawawarabi.webp?url";

/** デドロ字幕を使っている配信者（載せる許可をもらった人だけ。足すときは、アイコンを `assets/streamers/` に置いて 1 行足す）。`voice` は本人からもらったコメントで、言葉は変えない */
export const STREAMERS = [
  { name: "わっわらび", url: "https://www.twitch.tv/wawawawarabi", icon: wawawawarabi, voice: "キャラのアイコンが出てきて超かわいい！" },
  {
    name: "三毛森ギン",
    url: "https://www.twitch.tv/mikemorigin",
    icon: mikemorigin,
    voice: "今日コメントで精度高いって言われた！！！マジ凄い！神！日本語を拾う精度も高いみたい！！！サンキュー！",
  },
  { name: "川崎いすみ", url: "https://www.twitch.tv/kawasaki_isumi", icon: kawasakiIsumi, voice: "わーい！" },
];
