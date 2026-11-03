/** OBS のブラウザソースの既定の大きさ（操作パネルの案内と、オーバーレイの基準を同じ値にする） */
export const SUBTITLE_SIZE: [number, number] = [1000, 400];
export const RANK_SIZE: [number, number] = [400, 140];
export const ITEMS_SIZE: [number, number] = [420, 480];
/** アイテム説明のカード（deadlock-ui）の幅。既定のソースでは、カードを縮めてソースの幅に合わせる */
export const ITEM_CARD_WIDTH = 450;

/**
 * ブラウザソースの大きさと既定の大きさの比（幅と高さの小さい方。片方だけ変えても切れない）を求め、変わるたびに渡す。
 * OBS でソースを引き伸ばすと描いた絵を拡大するのでぼやけるが、プロパティの幅と高さを変えれば描き直すのでくっきりする。
 * 字幕は幅で折り返すので使わない
 */
export function fitSource([width, height]: [number, number], apply: (scale: number) => void) {
  const fit = () => {
    // 隠れているソースは大きさが 0 のことがある。0 倍にすると何も見えなくなるので、決まるまで待つ
    if (window.innerWidth === 0 || window.innerHeight === 0) return;
    apply(Math.min(window.innerWidth / width, window.innerHeight / height));
  };
  fit();
  window.addEventListener("resize", fit);
}
