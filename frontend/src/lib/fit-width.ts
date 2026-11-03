/**
 * ブラウザソースの幅に合わせて、ページ全体を拡大・縮小する（ランク・アイテム説明）。
 * OBS でソースを引き伸ばすと描いた絵を拡大するのでぼやけるが、幅を変えれば中身を描き直すのでくっきりする。
 * 字幕は幅で折り返すので使わない（幅を変えても文字の大きさは変えない）
 */
export function fitWidth(base: number) {
  const fit = () => {
    document.documentElement.style.zoom = String(window.innerWidth / base);
  };
  fit();
  window.addEventListener("resize", fit);
}
