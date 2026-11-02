<script lang="ts">
  import { onMount } from "svelte";
  import { fade, scale } from "svelte/transition";
  import type { RankState } from "../../generated/RankState";
  import { connect } from "../../lib/ws";

  // URL パラメータ（DESIGN.md 12 章）。lang はランク名の言語
  const lang = new URLSearchParams(location.search).get("lang") ?? "ja";

  let rank = $state<RankState | undefined>();

  onMount(() =>
    connect(["rank"], (msg) => {
      if (msg.type === "rank.update") rank = msg.rank;
    }),
  );

  const name = $derived(rank ? (rank.names[lang] ?? rank.names.en ?? "") : "");
  // オブスキュラス（未ランク）はサブランクを出さない
  const label = $derived(rank && rank.subrank > 0 ? `${name} ${rank.subrank}` : name);
</script>

{#if rank}
  <!-- バッジが変わったら作り直してアニメーションさせる -->
  {#key rank.badge}
    <div class="rank" style:--rank-color={rank.color} in:scale={{ start: 0.8, duration: 400 }}>
      <img class="badge" src={rank.image} alt="" />
      <div class="text">
        <p class="name" {lang}>{label}</p>
        {#if rank.points !== undefined}
          {#key rank.points}
            <p class="points" in:fade={{ duration: 300 }}>
              {rank.points.toLocaleString("en-US")} <span class="unit">RP</span>
            </p>
          {/key}
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
    --points-size: 24px;
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
  .points {
    font-size: var(--points-size);
    font-variant-numeric: tabular-nums;
  }
  .unit {
    font-size: 0.7em;
    opacity: 0.85;
  }
</style>
