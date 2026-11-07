<script lang="ts">
  import { onMount } from "svelte";
  import { scale } from "svelte/transition";
  import type { Anchor } from "../../generated/Anchor";
  import type { RankContent } from "../../generated/RankContent";
  import type { RankState } from "../../generated/RankState";
  import type { RankStyle } from "../../generated/RankStyle";
  import { placeItems } from "../../lib/anchor";
  import { applyFontRules, rankFontRules } from "../../lib/overlay-fonts";
  import { fitSource, rankSize } from "../../lib/overlay-size";
  import RankBadge from "../../lib/RankBadge.svelte";
  import { connect } from "../../lib/ws";

  let rank = $state<RankState | undefined>();
  /** ランク名の言語と、出すもの。操作パネルで変え、サーバーから届く（名前がなければ英語） */
  let lang = $state("ja");
  /** 表示の設定が届くまでは描かない（設定と違う形で一瞬出さない） */
  let content = $state<RankContent | undefined>();
  let anchor = $state<Anchor>("top-left");
  let look = $state<RankStyle | undefined>();

  /** 操作パネルのプレビュー（`?preview`）では、アカウントを入れる前でも見本を出しておく。OBS には出さない */
  const SAMPLE: RankState | undefined = new URLSearchParams(location.search).has("preview")
    ? {
        badge: 0,
        tier: 7,
        subrank: 3,
        names: { ja: "アルカニスト", en: "Arcanist" },
        progress: { value: 1240, max: 2000 },
      }
    : undefined;
  const shown = $derived(rank ?? SAMPLE);

  // 大きさは --scale で描く（zoom だとポイントの数字のアニメーションの位置がずれる）
  $effect(() =>
    content
      ? fitSource(rankSize(content), (factor) =>
          document.documentElement.style.setProperty("--scale", String(factor)),
        )
      : undefined,
  );

  onMount(() =>
    connect(["rank", "display"], (msg) => {
      if (msg.type === "rank.update") rank = msg.rank ?? undefined;
      if (msg.type === "display") {
        applyFontRules(rankFontRules(msg.rankStyle.font));
        look = msg.rankStyle;
        lang = msg.rankLanguage;
        content = msg.rankContent;
        anchor = msg.rankAnchor;
      }
    }),
  );
</script>

<!-- ソースの箱の中で、操作パネルで選んだ位置に寄せる -->
<div class="stage" style:place-items={placeItems(anchor)}>
  {#if shown && content}
    <!-- バッジが変わったら作り直してアニメーションさせる -->
    {#key shown.badge}
      <div class="card" in:scale={{ start: 0.8, duration: 400 }}>
        <RankBadge rank={shown} {lang} {content} {look} />
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
