<script lang="ts">
  import { onMount } from "svelte";
  import { scale } from "svelte/transition";
  import type { Anchor } from "../../generated/Anchor";
  import type { RankContent } from "../../generated/RankContent";
  import type { RankState } from "../../generated/RankState";
  import type { RankStyle } from "../../generated/RankStyle";
  import { placeItems } from "../../lib/anchor";
  import { applyFontImports, fontImports } from "../../lib/overlay-fonts";
  import { fitSource, rankSize } from "../../lib/overlay-size";
  import RankBadge from "../../lib/RankBadge.svelte";
  import { api } from "../../lib/api";
  import { connect } from "../../lib/ws";

  let rank = $state<RankState | undefined>();
  /** ランク名の言語と、出すもの。操作パネルで変え、サーバーから届く（名前がなければ英語） */
  let lang = $state<string | undefined>();
  /** 表示の設定が届くまでは描かない（設定と違う形で一瞬出さない） */
  let content = $state<RankContent | undefined>();
  let anchor = $state<Anchor>("top-left");
  let look = $state<RankStyle | undefined>();

  /** 見本のアイコンを取れないとき（ネットにつながらないなど）の仮の形 */
  const FALLBACK_IMAGE =
    "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'%3E%3Cpath fill='%23a8a29e' d='M48 6 82 20v28c0 20-14 34-34 42C28 82 14 68 14 48V20Z'/%3E%3C/svg%3E";

  /** 操作パネルのプレビュー（`?preview`）では、アカウントを入れる前でも見本を出しておく。OBS には出さない */
  const preview = new URLSearchParams(location.search).has("preview");
  /** 見本は core が本物と同じ作り方で返す（ゲームの画像は配布物に入れず、取ってくる）。
   * 名前はランクの言語の分だけ取るので、言語を変えたら、届くまで前の言語の名前のまま出す */
  let sample = $state<{ rank: RankState; lang: string } | undefined>();
  const shown = $derived(rank && lang ? { rank, lang } : sample);

  $effect(() => {
    if (!preview || !lang) return;
    const want = lang;
    let stale = false;
    void api.rankSample(want).then((s) => {
      if (!stale) sample = { rank: { ...s, image: s.image ?? FALLBACK_IMAGE }, lang: want };
    });
    return () => (stale = true);
  });

  // 大きさは --scale で描く（zoom だとポイントの数字のアニメーションの位置がずれる）
  $effect(() =>
    content
      ? fitSource(rankSize(content), (factor) =>
          document.documentElement.style.setProperty("--scale", String(factor)),
        )
      : undefined,
  );

  onMount(() => {
    return connect(["rank", "display"], (msg) => {
      if (msg.type === "rank.update") rank = msg.rank ?? undefined;
      if (msg.type === "display") {
        applyFontImports(fontImports([msg.rankStyle.font]));
        look = msg.rankStyle;
        lang = msg.rankLanguage;
        content = msg.rankContent;
        anchor = msg.rankAnchor;
      }
    });
  });
</script>

<!-- ソースの箱の中で、操作パネルで選んだ位置に寄せる -->
<div class="stage" style:place-items={placeItems(anchor)}>
  {#if shown && content}
    <!-- バッジが変わったら作り直してアニメーションさせる -->
    {#key shown.rank.badge}
      <div class="card" in:scale={{ start: 0.8, duration: 400 }}>
        <RankBadge rank={shown.rank} lang={shown.lang} {content} {look} />
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
