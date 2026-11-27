<script lang="ts">
  // 買ったアイテムのオーバーレイ（DESIGN.md 18.5）。ゲームの画面から読めた買い物を、小さく並べて出す。
  // 説明は出さない（買った瞬間に視聴者が読めるのは数秒。説明はアイテム説明の役目）
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import type { Purchase } from "../../generated/Purchase";
  import type { PurchaseCardsView } from "../../generated/PurchaseCardsView";
  import { placeItems } from "../../lib/anchor";
  import { entityName, glossary, loadGlossary } from "../../lib/glossary.svelte";
  import { badge, clock, labels, percent, timing } from "../../lib/purchase-stats";
  import { connect } from "../../lib/ws";

  /** 操作パネルのプレビュー（`?preview`）では、買わなくても見本を出しておく。OBS には出さない */
  const preview = new URLSearchParams(location.search).has("preview");
  /** OBS の中で開いている間だけ、ゲームの画面を読んでもらう（iframe の中は除く。プレビューで読ませない） */
  const inObs = "obsstudio" in window && window.parent === window;

  /** 一度に並べる数。それより多く買ったら、新しいほう */
  const MAX_ITEMS = 3;
  /** 待っている買い物の上限。買ったときから大きく遅れないよう、古いものは捨てる */
  const MAX_QUEUE = 3;

  /** 見本の買った時刻（10:00。1 つ目には遅く、2 つ目には早い） */
  const SAMPLE_CLOCK = 600;
  /** 見本の数字（定番と、珍しいもの） */
  const SAMPLE_RATES = [
    { share: 0.62, boughtAtSeconds: 190 },
    { share: 0.012, boughtAtSeconds: 1110 },
  ];

  let view = $state<PurchaseCardsView | undefined>();
  /** 今出している買い物。`id` は、同じものを続けて買ったときにも出し直すための番号 */
  let current = $state<{ id: number; purchase: Purchase } | undefined>();
  const queue: Purchase[] = [];
  let count = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  /** 見本は辞書から作る（ゲームの画像は配布物に入れず、取ってきたものを使う） */
  const sample = $derived.by((): Purchase | undefined => {
    if (!preview || !glossary.ready) return undefined;
    const items = SAMPLE_RATES.flatMap((rate, i) => {
      const entity = glossary.items[i];
      return entity ? [{ item: entity.className, rate }] : [];
    });
    return { hero: glossary.heroes[0]?.[1].className ?? null, clockSeconds: SAMPLE_CLOCK, items };
  });
  const shown = $derived(current ?? (sample && { id: 0, purchase: sample }));

  const lang = $derived(view?.language ?? "en");
  const text = $derived(labels(lang));
  const hero = $derived(
    shown?.purchase.hero ? glossary.named("hero", shown.purchase.hero) : undefined,
  );
  const clockSeconds = $derived(shown?.purchase.clockSeconds);
  const rows = $derived(
    (shown?.purchase.items ?? []).slice(-MAX_ITEMS).map(({ item, rate }) => {
      const entity = glossary.named("item", item);
      return {
        key: item,
        name: entity ? entityName(entity, lang) : item,
        icon: entity?.icon,
        slot: entity?.slot,
        rate: view?.showRate ? rate : null,
        // 買った時刻が読めたときだけ、平均と比べる
        timing:
          rate && clockSeconds != null ? timing(clockSeconds, rate.boughtAtSeconds) : undefined,
      };
    }),
  );

  function next() {
    const purchase = queue.shift();
    current = purchase && { id: ++count, purchase };
    clearTimeout(timer);
    if (current) timer = setTimeout(next, (view?.holdSeconds ?? 6) * 1000);
  }

  onMount(() => {
    const disconnect = connect(
      ["purchase", "glossary", "display"],
      (msg) => {
        switch (msg.type) {
          case "purchase":
            queue.push(msg.purchase);
            if (queue.length > MAX_QUEUE) queue.shift();
            if (!current) next();
            break;
          case "glossary.updated":
            void loadGlossary();
            break;
          case "display":
            view = msg.purchaseCards;
            break;
        }
      },
      undefined,
      inObs,
    );
    return () => {
      clearTimeout(timer);
      disconnect();
    };
  });
</script>

{#if view}
  <!-- ソースの箱の中で、操作パネルで選んだ位置に寄せる。右に寄せたら右から出す -->
  <div class="stage" style:place-items={placeItems(view.anchor)}>
    {#each shown ? [shown] : [] as group (group.id)}
      <ul
        class="list"
        {lang}
        in:fly={{ x: view.anchor.endsWith("right") ? 24 : -24, duration: 250 }}
        out:fade={{ duration: 300 }}
      >
        {#each rows as row (row.key)}
          {@const mark = row.rate ? badge(row.rate.share) : undefined}
          <li class="row {row.slot ?? ''}">
            <!-- 右下のヒーローは、そのヒーローでの数字だという印 -->
            <div class="icons">
              {#if row.icon}<img class="icon" src={row.icon} alt="" />{/if}
              {#if row.rate && hero?.icon}<img class="hero" src={hero.icon} alt="" />{/if}
            </div>
            <div class="body">
              <p class="name">{row.name}</p>
              {#if row.rate}
                <dl class="stats">
                  <div>
                    <dd>{percent(row.rate.share)}</dd>
                    <dt>{text.share}</dt>
                  </div>
                  <div>
                    <dd>
                      {clock(row.rate.boughtAtSeconds)}
                      {#if row.timing}<span class="timing {row.timing}">{text[row.timing]}</span
                        >{/if}
                    </dd>
                    <dt>{text.boughtAt}</dt>
                  </div>
                  {#if mark}<span class="badge {mark}">{text[mark]}</span>{/if}
                </dl>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/each}
  </div>
{/if}

<style>
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  :global(:root) {
    --scale: 1;
  }
  /* 中身が箱より大きくても、選んだ位置に寄せる（はみ出した分が切れる） */
  .stage {
    position: fixed;
    inset: 0;
    display: grid;
    grid-template: minmax(0, 1fr) / minmax(0, 1fr);
    padding: calc(8px * var(--scale));
  }
  /* 入れ替わるときに前のものと重ねる */
  .list {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
    zoom: var(--scale);
    color: #f4f1ea;
  }
  /* 色はゲーム内と同じくスロット（武器 / バイタリティ / スピリット）で分ける */
  .row {
    --slot: #a8a29e;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 340px;
    box-sizing: border-box;
    padding: 10px 14px 10px 10px;
    border-left: 4px solid var(--slot);
    border-radius: 6px;
    background: rgb(18 20 24 / 0.88);
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.45);
  }
  .weapon {
    --slot: #ffb35c;
  }
  .vitality {
    --slot: #98e36b;
  }
  .spirit {
    --slot: #cfa2ff;
  }
  .icons {
    position: relative;
    width: 64px;
    height: 64px;
    flex: none;
  }
  .icon {
    width: 100%;
    height: 100%;
    border-radius: 6px;
  }
  .body {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 6px;
  }
  .name {
    margin: 0;
    overflow: hidden;
    font-size: 18px;
    font-weight: 700;
    line-height: 1.2;
    color: var(--slot);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 統計の行の右の端に置く（名前の幅を取らない） */
  .badge {
    margin-left: auto;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 700;
    white-space: nowrap;
  }
  .rare {
    background: #ffd84d;
    color: #2a2000;
  }
  .staple {
    background: rgb(255 255 255 / 0.16);
    color: #f4f1ea;
  }
  /* 数字を大きく、ラベルを小さく（文より一目で読める） */
  .stats {
    display: flex;
    align-items: center;
    gap: 14px;
    margin: 0;
  }
  .stats div {
    display: flex;
    flex-direction: column;
  }
  /* 「早い」「遅い」は、平均の時刻の横に付ける（何と比べて早いのかが分かる） */
  .stats dd {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 18px;
    font-weight: 700;
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
  }
  .stats dt {
    font-size: 11px;
    line-height: 1.3;
    color: rgb(244 241 234 / 0.7);
    white-space: nowrap;
  }
  .timing {
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    line-height: 1.4;
    white-space: nowrap;
  }
  .early {
    background: #7fd4ff;
    color: #00263a;
  }
  .late {
    background: #ff9b7a;
    color: #3a0e00;
  }
  .hero {
    position: absolute;
    right: -8px;
    bottom: -8px;
    width: 30px;
    height: 30px;
    filter: drop-shadow(0 1px 2px rgb(0 0 0 / 0.8));
  }
</style>
