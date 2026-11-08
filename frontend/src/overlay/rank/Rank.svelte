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
   * 名前は英語とランクの言語の分だけ取るので、言語を変えたら、届くまで前の言語の名前のまま出す */
  let sample = $state<RankState | undefined>();
  let sampleLang = $state<string | undefined>();
  const shown = $derived(rank ?? (sample && { ...sample, image: sample.image ?? FALLBACK_IMAGE }));
  let request: { lang: string; controller: AbortController } | undefined;

  /** 本物のランクがあれば取らない。取れなかった名前とアイコンは、次に display が届いたときに取り直す */
  function loadSample(want: string) {
    if (!preview || rank) return;
    if (sample?.names[want] && sample.image) {
      sampleLang = want;
      return;
    }
    if (request?.lang === want) return;
    // 取り消しても core は取りかけたものを最後まで取って覚えるので、前の言語の名前も無駄にならない
    request?.controller.abort();
    const controller = new AbortController();
    request = { lang: want, controller };
    api
      .rankSample(want, controller.signal)
      .then((s) => {
        sample = s;
        sampleLang = want;
      })
      .catch((e: unknown) => {
        if (!controller.signal.aborted) console.warn("rank sample", e);
      })
      .finally(() => {
        if (request?.controller === controller) request = undefined;
      });
  }

  // 大きさは --scale で描く（zoom だとポイントの数字のアニメーションの位置がずれる）
  $effect(() =>
    content
      ? fitSource(rankSize(content), (factor) =>
          document.documentElement.style.setProperty("--scale", String(factor)),
        )
      : undefined,
  );

  onMount(() => {
    // 表示の言語が届くのを待たずに、英語の名前とアイコンを取り始める
    loadSample("en");
    const disconnect = connect(["rank", "display"], (msg) => {
      if (msg.type === "rank.update") rank = msg.rank ?? undefined;
      if (msg.type === "display") {
        applyFontImports(fontImports([msg.rankStyle.font]));
        look = msg.rankStyle;
        lang = msg.rankLanguage;
        content = msg.rankContent;
        anchor = msg.rankAnchor;
        loadSample(msg.rankLanguage);
      }
    });
    return () => {
      disconnect();
      request?.controller.abort();
    };
  });
</script>

<!-- ソースの箱の中で、操作パネルで選んだ位置に寄せる -->
<div class="stage" style:place-items={placeItems(anchor)}>
  {#if shown && content}
    <!-- バッジが変わったら作り直してアニメーションさせる -->
    {#key shown.badge}
      <div class="card" in:scale={{ start: 0.8, duration: 400 }}>
        <RankBadge rank={shown} lang={(rank ? lang : sampleLang) ?? "en"} {content} {look} />
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
