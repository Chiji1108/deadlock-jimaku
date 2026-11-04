<script lang="ts">
  // ランクの見た目。OBS のオーバーレイと LP の見本で同じものを使う
  import { TextMorph } from "torph/svelte";
  import type { LeaderboardRank } from "../generated/LeaderboardRank";
  import type { RankContent } from "../generated/RankContent";
  import type { RankState } from "../generated/RankState";
  import { REGIONS } from "./regions";

  let { rank, lang, content }: { rank: RankState; lang: string; content: RankContent } = $props();

  /** リーダーボードの順位。ランク名と同じ言語にそろえる（日本語は「アジア 12位」、ほかは「#12 Asia」） */
  const placement = (l: LeaderboardRank) =>
    shown === "ja"
      ? `${REGIONS[l.region].ja} ${number(l.rank)}位`
      : `#${number(l.rank)} ${REGIONS[l.region].en}`;

  /** ランク名を出す言語（選んだ言語の名前がなければ英語） */
  const shown = $derived(rank.names[lang] ? lang : "en");
  const label = $derived.by(() => {
    const name = rank.names[shown];
    // 未ランクはサブランクを出さない。名前がまだ取れていなければ数字だけにしない。
    // サブランクはゲームと同じローマ数字
    const subrank = ["I", "II", "III", "IV", "V", "VI"][rank.subrank - 1];
    return [name, name && rank.subrank > 0 ? subrank : undefined].filter(Boolean).join(" ");
  });
  const ratio = $derived(rank.progress ? rank.progress.value / rank.progress.max : 0);
  const number = (n: number) => n.toLocaleString("en-US");
</script>

<div class="rank">
  {#if rank.image}<img class="badge" src={rank.image} alt="" />{/if}
  <!-- 1 行目: ランク名、2 行目: 太いゲージ（ポイントは中に重ねる）。出すものは操作パネルで選ぶ -->
  {#if content !== "icon"}<div class="text">
      {#if label}<p class="name" lang={shown}>{label}</p>{/if}
      {#if content !== "full"}
        <!-- 名前だけ -->
      {:else if rank.leaderboard}
        <!-- エターナスは上のティアがないので、リーダーボードの順位を出す -->
        <p class="points">{placement(rank.leaderboard)}</p>
      {:else if rank.progress}
        <div class="bar">
          <div class="fill" style:width="{ratio * 100}%"></div>
          <p class="points">
            <!-- ポイントが変わったら桁ごとに数字を動かす -->
            <TextMorph text={number(rank.progress.value)} numbers as="span" duration={600} /><span
              class="max">{` / ${number(rank.progress.max)}`}</span
            >
          </p>
        </div>
      {:else if rank.points !== undefined}
        <!-- 順位が取れないエターナスは、累積のポイントを出す -->
        <p class="points">
          <TextMorph text={number(rank.points)} numbers as="span" duration={600} />
          <span class="unit">RP</span>
        </p>
      {/if}
    </div>{/if}
</div>

<style>
  /* OBS の「カスタム CSS」で上書きできるよう、見た目は変数にまとめる。
     大きさは既定のソース（rankSize）での px に、ソースの大きさとの比（--scale）を掛ける。
     --scale は親で変えられるよう、掛け算は使う場所に置く */
  :global(:root) {
    --scale: 1;
    --text-color: #fff;
    --gauge-color: rgb(255 255 255 / 0.9);
    --outline-color: rgb(0 0 0 / 0.85);
  }
  .rank {
    display: inline-flex;
    align-items: center;
    gap: calc(14px * var(--scale));
    padding: calc(8px * var(--scale));
    color: var(--text-color);
    -webkit-text-stroke: 0.16em var(--outline-color);
    paint-order: stroke fill;
    text-shadow: 0 calc(2px * var(--scale)) calc(6px * var(--scale)) rgb(0 0 0 / 0.6);
  }
  .badge {
    width: var(--badge-size, calc(96px * var(--scale)));
    height: var(--badge-size, calc(96px * var(--scale)));
    object-fit: contain;
    filter: drop-shadow(0 calc(2px * var(--scale)) calc(6px * var(--scale)) rgb(0 0 0 / 0.6));
  }
  p {
    margin: 0;
    font-weight: 800;
    line-height: 1.2;
  }
  /* ティアの色はゲームの中で見かけないので使わず、字幕と同じく白に黒の縁取り */
  .name {
    font-size: var(--name-size, calc(34px * var(--scale)));
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: calc(6px * var(--scale));
  }
  .bar {
    position: relative;
    display: grid;
    place-items: center;
    min-width: var(--gauge-width, calc(260px * var(--scale)));
    height: var(--gauge-height, calc(30px * var(--scale)));
    border-radius: 999px;
    background: rgb(0 0 0 / 0.55);
    box-shadow: 0 0 0 calc(2px * var(--scale)) rgb(0 0 0 / 0.6);
    overflow: hidden;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--gauge-color);
    transition: width 0.6s ease-out;
  }
  /* 白い塗りの上でも暗い地の上でも読めるよう、縁取りした白い文字を中央に重ねる */
  .bar .points {
    position: relative;
    line-height: 1;
  }
  .points {
    font-size: var(--points-size, calc(20px * var(--scale)));
    font-variant-numeric: tabular-nums;
  }
  .unit,
  .max {
    font-size: 0.7em;
    opacity: 0.75;
  }
</style>
