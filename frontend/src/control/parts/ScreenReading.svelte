<script lang="ts">
  // 買ったアイテムのタブの「読み取りの状態」（DESIGN.md 18.5）。読めていなくても、配信の間は何も出ないだけで
  // 気づけないので、配信の前にここを見れば分かるようにする。読むのは、オーバーレイが OBS につながっている間だけで、
  // ここは、その結果を見せるだけ（タブを開いただけでは、ゲームの画面を読み始めない）
  import * as Card from "$lib/components/ui/card";
  import type { ScreenStatus } from "../../generated/ScreenStatus";
  import { entityName, glossary } from "../../lib/glossary.svelte";
  import { clock } from "../../lib/purchase-stats";
  import { connect } from "../../lib/ws";
  import { STATUS_DOTS, panel } from "../panel.svelte";
  import Help from "./Help.svelte";

  /** 配置を確かめた画面の高さ。ほかの大きさでも止めずに読み、読めていればそれでよい */
  const CHECKED_HEIGHT = 1080;

  let status = $state<ScreenStatus>({ state: "off" });

  $effect(() => {
    if (panel.tab !== "purchases") return;
    return connect(["screen"], (msg) => {
      if (msg.type === "screen.status") status = msg.status;
    });
  });

  const lang = $derived(panel.settings!.micLanguage);
  /** 上部バーの枠ごとのヒーロー。読めない枠は undefined */
  const heroes = $derived(
    status.state === "reading"
      ? status.heroes.map((name) => (name ? glossary.named("hero", name) : undefined))
      : [],
  );
  const items = $derived(
    status.state === "reading"
      ? status.items.flatMap((name) => glossary.named("item", name) ?? [])
      : [],
  );
  const own = $derived(
    status.state === "reading" && status.own ? glossary.named("hero", status.own) : undefined,
  );
  /** 上部バーもアイテム欄も読めていない。試合の外か、何かに隠れているか、読めない環境 */
  const nothing = $derived(
    status.state === "reading" && heroes.every((hero) => !hero) && items.length === 0,
  );

  const summary = $derived.by((): { tone: keyof typeof STATUS_DOTS; label: string } => {
    switch (status.state) {
      case "off":
        return { tone: "idle", label: "読んでいません（OBS に追加すると、読み始めます）" };
      case "preparing":
        return { tone: "busy", label: "見本の画像を用意しています（初めは 13 MB ほど取ります）" };
      case "noWindow":
        return { tone: "idle", label: "Deadlock が開いていません" };
      case "failed":
        return { tone: "error", label: `読めません（${status.detail}）` };
      case "reading":
        if (nothing) return { tone: "warning", label: "試合の画面が読めていません" };
        // 観戦中は、左下が相手の持ち物になるので、買ったと数えない
        if (status.spectating)
          return { tone: "live", label: "読めています（観戦中。買ったアイテムは出しません）" };
        return {
          tone: "live",
          label: status.shop ? "読めています（ショップを開いています）" : "読めています",
        };
    }
  });
</script>

<Card.Root>
  <Card.Header>
    <Card.Title class="flex items-center gap-1.5"
      >読み取りの状態<Help
        >いま読めているヒーローとアイテムです。ゲームの画面と見比べて、合っていれば動いています。オーバーレイを
        OBS に追加して、サンドボックスに入れば、配信の前に試せます</Help
      ></Card.Title
    >
  </Card.Header>
  <Card.Content class="flex flex-col gap-4">
    <p class="flex items-center gap-2 text-sm">
      <span class="size-2 shrink-0 rounded-full {STATUS_DOTS[summary.tone]}"></span>
      {summary.label}
    </p>
    {#if status.state === "reading"}
      {#if nothing}
        <div class="text-sm text-muted-foreground">
          <p>試合やサンドボックスの中なら、次のどれかかもしれません</p>
          <ul class="mt-1 list-disc pl-5">
            <li>ヒーローやアイテム、HUD の見た目を変える MOD を入れている</li>
            <li>ゲームの設定で、HUD の大きさを変えている</li>
            <li>スコアボードやメニューなど、ほかの画面が重なっている</li>
            <li>ゲームのアップデートで、画面の見た目が変わった</li>
          </ul>
        </div>
      {:else}
        <div class="flex flex-col gap-1.5">
          <p class="text-xs text-muted-foreground">
            上部バーのヒーロー（上の段が左のチーム、下の段が右のチーム。読めない枠は「?」）
          </p>
          <ul class="grid w-fit grid-cols-6 gap-1">
            {#each heroes as hero, slot (slot)}
              <li
                class="flex size-9 items-center justify-center overflow-hidden rounded-md border bg-muted/40 text-xs text-muted-foreground"
                title={hero ? entityName(hero, lang) : "読めない枠"}
              >
                {#if hero?.icon}
                  <img
                    class="size-full object-cover"
                    src={hero.icon}
                    alt={entityName(hero, lang)}
                  />
                {:else}
                  ?
                {/if}
              </li>
            {/each}
          </ul>
        </div>
        <!-- 購入率は自分のヒーローの数字、「早い」「遅い」は試合の時計から出す。どちらも読めないと出ない -->
        <dl class="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-1.5 text-sm">
          <dt class="text-xs text-muted-foreground">自分のヒーロー</dt>
          <dd class="flex items-center gap-1.5">
            {#if own}
              {#if own.icon}<img class="size-6 rounded" src={own.icon} alt="" />{/if}
              {entityName(own, lang)}
            {:else}
              <span class="text-muted-foreground"
                >まだ分かりません（アビリティを取ると分かります）</span
              >
            {/if}
          </dd>
          <dt class="text-xs text-muted-foreground">試合の時計</dt>
          <dd class="tabular-nums">
            {#if status.clock != null}
              {clock(status.clock)}
            {:else}
              <span class="text-muted-foreground">読めていません</span>
            {/if}
          </dd>
        </dl>
        <div class="flex flex-col gap-1.5">
          <p class="text-xs text-muted-foreground">
            {status.spectating
              ? "観戦している相手のアイテム"
              : "持っているアイテム"}（{items.length} 個）
          </p>
          <ul class="flex min-h-9 flex-wrap gap-1">
            {#each items as item (item.className)}
              <li class="size-9 overflow-hidden rounded-md border" title={entityName(item, lang)}>
                {#if item.icon}
                  <img class="size-full" src={item.icon} alt={entityName(item, lang)} />
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/if}
      {#if status.height !== CHECKED_HEIGHT}
        <p class="text-xs text-muted-foreground">
          この画面の大きさ（{status.width} × {status.height}）は、まだ十分に確かめていません。上のヒーローとアイテムが合っていれば、動いています
        </p>
      {/if}
    {/if}
  </Card.Content>
</Card.Root>
