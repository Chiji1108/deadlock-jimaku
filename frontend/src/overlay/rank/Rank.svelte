<script lang="ts">
  import { onMount } from "svelte";
  import { scale } from "svelte/transition";
  import type { Anchor } from "../../generated/Anchor";
  import type { RankContent } from "../../generated/RankContent";
  import type { RankState } from "../../generated/RankState";
  import { placeItems } from "../../lib/anchor";
  import { fitSource, rankSize } from "../../lib/overlay-size";
  import RankBadge from "../../lib/RankBadge.svelte";
  import { connect } from "../../lib/ws";

  let rank = $state<RankState | undefined>();
  /** ランク名の言語と、出すもの。操作パネルで変え、サーバーから届く（名前がなければ英語） */
  let lang = $state("ja");
  let content = $state<RankContent>("full");
  let anchor = $state<Anchor>("top-left");

  // 大きさは --scale で描く（zoom だとポイントの数字のアニメーションの位置がずれる）
  $effect(() =>
    fitSource(rankSize(content), (factor) =>
      document.documentElement.style.setProperty("--scale", String(factor)),
    ),
  );

  onMount(() =>
    connect(["rank", "display"], (msg) => {
      if (msg.type === "rank.update") rank = msg.rank ?? undefined;
      if (msg.type === "display") {
        lang = msg.rankLanguage;
        content = msg.rankContent;
        anchor = msg.rankAnchor;
      }
    }),
  );
</script>

<!-- ソースの箱の中で、操作パネルで選んだ位置に寄せる -->
<div class="stage" style:place-items={placeItems(anchor)}>
  {#if rank}
    <!-- バッジが変わったら作り直してアニメーションさせる -->
    {#key rank.badge}
      <div class="card" in:scale={{ start: 0.8, duration: 400 }}>
        <RankBadge {rank} {lang} {content} />
      </div>
    {/key}
  {/if}
</div>

<style>
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  /* 中身が箱より大きくても、選んだ位置に寄せる（はみ出した分が切れる） */
  .stage {
    position: fixed;
    inset: 0;
    display: grid;
    grid-template: minmax(0, 1fr) / minmax(0, 1fr);
  }
  .card {
    display: inline-flex;
  }
</style>
