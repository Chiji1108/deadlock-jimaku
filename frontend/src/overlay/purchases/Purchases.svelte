<script lang="ts">
  // 買ったアイテムのオーバーレイ（DESIGN.md 18.5）。ゲームの画面から読めた買い物を、小さく並べて出す。
  // 説明は出さない（買った瞬間に視聴者が読めるのは数秒。説明はアイテム説明の役目）
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import type { Purchase } from "../../generated/Purchase";
  import type { PurchaseCardsView } from "../../generated/PurchaseCardsView";
  import { placeItems } from "../../lib/anchor";
  import { entityName, glossary, loadGlossary } from "../../lib/glossary.svelte";
  import { samplePurchase } from "../../lib/purchase-sample";
  import { badge, clock, labels, percent, timing } from "../../lib/purchase-stats";
  import { connect, inObs } from "../../lib/ws";

  /** 操作パネルのプレビュー（`?preview`）では、買わなくても見本を出しておく。OBS には出さない */
  const preview = new URLSearchParams(location.search).has("preview");

  /** 一度に並べる数。それより多く買ったら、新しいほう */
  const MAX_ITEMS = 3;
  /** 待っている買い物の上限。買ったときから大きく遅れないよう、古いものは捨てる */
  const MAX_QUEUE = 3;

  let view = $state<PurchaseCardsView | undefined>();
  /** 今出している買い物。`id` は、同じものを続けて買ったときにも出し直すための番号 */
  let current = $state<{ id: number; purchase: Purchase } | undefined>();
  const queue: Purchase[] = [];
  let count = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const sample = $derived(preview ? samplePurchase() : undefined);
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
        // 統計は、数字、その数字が目立つときの一言、何の数字か、の組で出す
        stats:
          view?.showRate && rate
            ? [
                { value: percent(rate.share), tag: badge(rate.share), label: text.share },
                {
                  value: clock(rate.boughtAtSeconds),
                  // 買った時刻が読めたときだけ、平均と比べる
                  tag:
                    clockSeconds == null ? undefined : timing(clockSeconds, rate.boughtAtSeconds),
                  label: text.boughtAt,
                },
              ]
            : [],
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
      inObs() ? "purchases" : undefined,
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
        <!-- 誰が買ったか。下の数字が、このヒーローでの数字だという印になる。「購入」とは書かない
             （下に「購入率」と出るし、言葉が無ければ、どの言語の配信でも通じる）。自分のヒーローがまだ分からなければ、出さない -->
        {#if hero}
          <li class="head">
            {#if hero.icon}<img src={hero.icon} alt="" />{/if}
            {entityName(hero, lang)}
          </li>
        {/if}
        {#each rows as row (row.key)}
          <li class="row {row.slot ?? ''}">
            {#if row.icon}<img class="icon" src={row.icon} alt="" />{/if}
            <div class="body">
              <p class="name">{row.name}</p>
              {#if row.stats.length}
                <dl class="stats">
                  {#each row.stats as stat (stat.label)}
                    <div>
                      <dd>
                        {stat.value}
                        {#if stat.tag}<span class="tag {stat.tag}">{text[stat.tag]}</span>{/if}
                      </dd>
                      <dt>{stat.label}</dt>
                    </div>
                  {/each}
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
  .icon {
    width: 64px;
    height: 64px;
    flex: none;
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
  /* 数字を大きく、ラベルを小さく（文より一目で読める） */
  .stats {
    display: flex;
    align-items: center;
    gap: 18px;
    margin: 0;
  }
  .stats div {
    display: flex;
    flex-direction: column;
  }
  /* 並べたときに、2 つ目の数字の位置がそろうように、1 つ目の幅を決めておく */
  .stats div:first-child {
    min-width: 104px;
  }
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
  /* その数字が目立つときの一言。数字のすぐ横に置く（何についての一言かが分かる） */
  .tag {
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    line-height: 1.4;
    white-space: nowrap;
  }
  .staple {
    background: rgb(255 255 255 / 0.18);
    color: #f4f1ea;
  }
  .rare {
    background: #ffd84d;
    color: #2a2000;
  }
  .early {
    background: #7fd4ff;
    color: #00263a;
  }
  .late {
    background: #ff9b7a;
    color: #3a0e00;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    padding: 3px 12px 3px 6px;
    border-radius: 999px;
    background: rgb(18 20 24 / 0.88);
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.45);
    font-size: 13px;
    font-weight: 700;
    line-height: 1.2;
  }
  .head img {
    width: 24px;
    height: 24px;
  }
</style>
