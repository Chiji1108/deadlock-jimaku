<script lang="ts">
  import { onMount } from "svelte";
  import { scale } from "svelte/transition";
  import { TextMorph } from "torph/svelte";
  import type { RankState } from "../../generated/RankState";
  import { connect } from "../../lib/ws";

  let rank = $state<RankState | undefined>();
  /** ランク名の言語。操作パネルで変え、サーバーから届く（名前がなければ英語） */
  let lang = $state("ja");

  onMount(() =>
    connect(["rank", "display"], (msg) => {
      if (msg.type === "rank.update") rank = msg.rank ?? undefined;
      if (msg.type === "display") lang = msg.rankLanguage;
    }),
  );

  const label = $derived.by(() => {
    if (!rank) return "";
    const name = rank.names[lang] ?? rank.names.en;
    // 未ランクはサブランクを出さない。名前がまだ取れていなければ数字だけにしない
    return [name, name && rank.subrank > 0 ? String(rank.subrank) : undefined]
      .filter(Boolean)
      .join(" ");
  });
  const ratio = $derived(rank?.progress ? rank.progress.value / rank.progress.max : 0);
  const number = (n: number) => n.toLocaleString("en-US");
</script>

{#if rank}
  <!-- バッジが変わったら作り直してアニメーションさせる -->
  {#key rank.badge}
    <div class="rank" style:--rank-color={rank.color} in:scale={{ start: 0.8, duration: 400 }}>
      {#if rank.image}<img class="badge" src={rank.image} alt="" />{/if}
      <!-- 1 行目: ランク名、2 行目: 太いゲージ（ポイントは中に重ねる） -->
      <div class="text">
        {#if label}<p class="name" {lang}>{label}</p>{/if}
        {#if rank.progress}
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
          <!-- エターナスは上位何 % かで決まるのでゲージなし。累積のポイントだけ出す -->
          <p class="points">
            <TextMorph text={number(rank.points)} numbers as="span" duration={600} />
            <span class="unit">RP</span>
          </p>
        {/if}
      </div>
    </div>
  {/key}
{/if}

<style>
  /* OBS の「カスタム CSS」で上書きできるよう、見た目は変数にまとめる */
  :global(:root) {
    --badge-size: 96px;
    --name-size: 34px;
    --points-size: 20px;
    --gauge-width: 260px;
    --gauge-height: 30px;
    --text-color: #fff;
    --outline-color: rgb(0 0 0 / 0.85);
  }
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  .rank {
    display: inline-flex;
    align-items: center;
    gap: 14px;
    padding: 8px;
    color: var(--text-color);
    -webkit-text-stroke: 0.16em var(--outline-color);
    paint-order: stroke fill;
    text-shadow: 0 2px 6px rgb(0 0 0 / 0.6);
  }
  .badge {
    width: var(--badge-size);
    height: var(--badge-size);
    object-fit: contain;
    filter: drop-shadow(0 2px 6px rgb(0 0 0 / 0.6));
  }
  p {
    margin: 0;
    font-weight: 800;
    line-height: 1.2;
  }
  .name {
    font-size: var(--name-size);
    color: var(--rank-color, var(--text-color));
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .bar {
    position: relative;
    display: grid;
    place-items: center;
    min-width: var(--gauge-width);
    height: var(--gauge-height);
    border-radius: 999px;
    background: rgb(0 0 0 / 0.55);
    box-shadow: 0 0 0 2px rgb(0 0 0 / 0.6);
    overflow: hidden;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--rank-color, var(--text-color));
    transition: width 0.6s ease-out;
  }
  /* ゲージの色の上でも暗い地の上でも読めるよう、縁取りした白い文字を中央に重ねる */
  .bar .points {
    position: relative;
    line-height: 1;
  }
  .points {
    font-size: var(--points-size);
    font-variant-numeric: tabular-nums;
  }
  .unit,
  .max {
    font-size: 0.7em;
    opacity: 0.75;
  }
</style>
