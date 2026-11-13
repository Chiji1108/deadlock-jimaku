<script lang="ts">
  import CaptionsIcon from "@lucide/svelte/icons/captions";
  import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
  import GiftIcon from "@lucide/svelte/icons/gift";
  import KeyRoundIcon from "@lucide/svelte/icons/key-round";
  import MicIcon from "@lucide/svelte/icons/mic";
  import PackageIcon from "@lucide/svelte/icons/package";
  import SquareIcon from "@lucide/svelte/icons/square";
  import TrophyIcon from "@lucide/svelte/icons/trophy";
  import WalletIcon from "@lucide/svelte/icons/wallet";
  import UserRoundIcon from "@lucide/svelte/icons/user-round";
  import WorkflowIcon from "@lucide/svelte/icons/workflow";
  import { ModeWatcher } from "mode-watcher";
  import { onMount } from "svelte";
  import { MediaQuery } from "svelte/reactivity";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import { Toaster } from "$lib/components/ui/sonner";
  import * as Tabs from "$lib/components/ui/tabs";
  import AboutDialog from "./parts/AboutDialog.svelte";
  import Onboarding from "./parts/Onboarding.svelte";
  import BetaDialog from "./parts/BetaDialog.svelte";
  import WhatsNew from "./parts/WhatsNew.svelte";
  import { fallbackText, panel, transcriptionErrorText } from "./panel.svelte";
  import { total, unpriced, usd } from "./usage";
  import AudioTab from "./tabs/AudioTab.svelte";
  import DisplayTab from "./tabs/DisplayTab.svelte";
  import EnginesTab from "./tabs/EnginesTab.svelte";
  import PeopleTab from "./tabs/PeopleTab.svelte";
  import ItemsTab from "./tabs/ItemsTab.svelte";
  import KeysTab from "./tabs/KeysTab.svelte";
  import RankTab from "./tabs/RankTab.svelte";
  import ShareTab from "./tabs/ShareTab.svelte";
  import UsageTab from "./tabs/UsageTab.svelte";
  import { CHROME, chromeFinals, enterKeys, keyNames } from "../lib/engines";

  onMount(() => panel.start());

  const s = $derived(panel.settings);
  /** 中身の欄（広い画面ではここだけがスクロールする） */
  let content = $state<HTMLElement>();

  const TABS = [
    { value: "audio", label: "マイクと翻訳", icon: MicIcon, page: AudioTab },
    { value: "engines", label: "モード", icon: WorkflowIcon, page: EnginesTab },
    { value: "people", label: "プロフィール", icon: UserRoundIcon, page: PeopleTab },
    {
      value: "display",
      label: "字幕",
      icon: CaptionsIcon,
      page: DisplayTab,
      heading: "オーバーレイ",
      wide: true,
    },
    { value: "items", label: "アイテム説明", icon: PackageIcon, page: ItemsTab, wide: true },
    { value: "rank", label: "ランク", icon: TrophyIcon, page: RankTab, wide: true },
    {
      value: "keys",
      label: "API キー",
      icon: KeyRoundIcon,
      page: KeysTab,
      heading: "キーと料金",
    },
    { value: "usage", label: "料金", icon: WalletIcon, page: UsageTab },
    // 非推奨で、将来なくなるので、一番下に置く（DESIGN.md 17.1）
    { value: "share", label: "おすそ分け", icon: GiftIcon, page: ShareTab, deprecated: true },
  ];
  // デスクトップのウィンドウでは左のサイドバー。OBS のドックのように狭いときは上に並べる
  const wide = new MediaQuery("min-width: 768px");
  /** オーバーレイのタブはプレビューと設定を 2 列に並べるので広く取る。ほかは読みやすい幅に留める */
  const pageWidth = $derived(
    TABS.find((t) => t.value === panel.tab)?.wide ? "max-w-6xl" : "max-w-3xl",
  );

  // タブを変えたら（サイドバーからでも、料金や注意のリンクからでも）先頭から見せる。
  // おすそ分けで使われた額はサーバーにあるので、それを出すタブを開いたら取り直す
  $effect(() => {
    if (panel.tab === "share" || panel.tab === "usage") void panel.refreshShare();
    content?.scrollTo({ top: 0 });
    window.scrollTo({ top: 0 });
  });

  /** 確定の文を Chrome が作るなら（Chrome のモード）、字幕は Chrome のウィンドウが動いて出る */
  function chromeStatus() {
    if (panel.chromeStatus?.state === "failed")
      return { label: "止まっています", dot: "bg-destructive" };
    if (panel.chromeStatus?.state === "missing")
      return { label: "Google Chrome が必要です", dot: "bg-destructive" };
    if (!panel.chromeStatus)
      return { label: "Chrome のウィンドウが開いていません", dot: "bg-amber-500" };
    if (panel.chromeStatus.state === "blocked")
      return { label: "再開を待っています", dot: "bg-amber-500" };
    if (panel.chromeStatus.state === "listening")
      return { label: "字幕を出しています", dot: "bg-emerald-500 animate-pulse" };
    return { label: "準備しています…", dot: "bg-sky-500 animate-pulse" };
  }

  const status = $derived.by(() => {
    if (!s?.transcriptionEnabled) return { label: "停止中", dot: "bg-muted-foreground/50" };
    if (panel.transcriptionError) return { label: "止まっています", dot: "bg-destructive" };
    if (panel.preparing) {
      const gb = (bytes: number) => (bytes / 1e9).toFixed(1);
      const d = panel.downloading;
      return {
        label: d?.total
          ? `PC の中のモデルをダウンロードしています（${gb(d.done)} / ${gb(d.total)} GB）…`
          : "PC の中のモデルを準備しています（初回はダウンロードします）…",
        dot: "bg-sky-500 animate-pulse",
      };
    }
    if (panel.silent) return { label: "音が届いていません", dot: "bg-amber-500" };
    if (chromeFinals(panel.view!.running)) return chromeStatus();
    if (panel.level === undefined)
      return { label: "準備しています…", dot: "bg-sky-500 animate-pulse" };
    return { label: "字幕を出しています", dot: "bg-emerald-500 animate-pulse" };
  });
</script>

<ModeWatcher />
<Toaster richColors position="bottom-right" />

<div
  class="flex min-h-screen flex-col bg-background text-foreground has-data-[orientation=vertical]:h-screen"
>
  <header class="sticky top-0 z-10 shrink-0 border-b bg-background/80 backdrop-blur">
    <!-- 狭いときはボタンを次の行に回す -->
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
      <!-- 名前はウィンドウ（OBS ならドックのタブ）に出ているので、状態と今回の料金を置く -->
      <h1 class="sr-only">デドロ字幕</h1>
      <div class="flex min-w-40 flex-1 flex-col gap-0.5">
        <p class="flex items-center gap-2 text-sm font-medium">
          <span class="size-2 shrink-0 rounded-full {status.dot}"></span>
          {panel.server === "open" ? status.label : "アプリに接続しています…"}
        </p>
        <!-- 自分で払う分があるときだけ出す（無料のモードや、コードで使うだけなら、増えていく額を見せない）。使用量が届く前も行の高さを取っておく（届いたときに帯の高さが変わらないように） -->
        {#if s && panel.pays}
          <p class="h-4 text-xs text-muted-foreground tabular-nums">
            {#if panel.server === "open" && panel.usage}
              <button
                class="hover:text-foreground hover:underline"
                onclick={() => (panel.tab = "usage")}
              >
                起動してからの料金 {usd(total(panel.usage.session))}{unpriced(panel.usage.session)
                  ? " ＋α"
                  : ""}
              </button>
            {/if}
          </p>
        {/if}
      </div>
      {#if s}
        {#if s.transcriptionEnabled}
          <Button variant="outline" onclick={() => panel.setTranscription(false)}>
            <SquareIcon />字幕を止める
          </Button>
        {:else if panel.needsKey}
          <div class="flex flex-wrap gap-2">
            <Button variant="outline" onclick={() => (panel.tab = "keys")}
              >{enterKeys(panel.missingKeys)}</Button
            >
            <Button onclick={panel.startWithChrome}>無料の Chrome モードで始める</Button>
          </div>
        {:else}
          <Button onclick={() => panel.setTranscription(true)}>
            <MicIcon />字幕を始める
          </Button>
        {/if}
      {/if}
    </div>
  </header>

  {#if s}
    <WhatsNew />
    <!-- 「おすすめ」は、どのタブからも選べる（モードのタブ、マイクと翻訳の提案、更新の知らせ） -->
    <BetaDialog />
    <Tabs.Root
      bind:value={panel.tab}
      orientation={wide.current ? "vertical" : "horizontal"}
      class="min-h-0 flex-1 gap-0"
    >
      <!-- 縦のとき（サイドバー）は高さいっぱいに伸ばし、入りきらなければサイドバーだけスクロールする -->
      <div
        class="flex shrink-0 flex-col px-4 pt-4 group-data-vertical/tabs:w-48 group-data-vertical/tabs:overflow-y-auto group-data-vertical/tabs:border-r group-data-vertical/tabs:bg-muted/30 group-data-vertical/tabs:p-3"
      >
        <Tabs.List
          class="w-full shrink-0 justify-start overflow-x-auto overflow-y-hidden group-data-vertical/tabs:gap-1 group-data-vertical/tabs:bg-transparent"
        >
          {#each TABS as t (t.value)}
            <!-- サイドバーでは種類ごとに分ける（heading が空なら区切りの余白だけ） -->
            {#if t.heading !== undefined}
              <p
                role="presentation"
                class="self-stretch px-[15px] pt-3 text-xs text-muted-foreground group-data-horizontal/tabs:hidden"
              >
                {t.heading}
              </p>
            {/if}
            <!-- 横に並べるときは幅が狭いので、アイコンを出さない（入りきらなければ横にスクロール） -->
            <Tabs.Trigger
              value={t.value}
              class="group-data-horizontal/tabs:[&>svg]:hidden group-data-vertical/tabs:h-9 group-data-vertical/tabs:flex-none group-data-vertical/tabs:px-3"
            >
              <t.icon />{t.label}
              {#if t.deprecated}
                <span class="text-[10px] text-muted-foreground">非推奨</span>
              {/if}
            </Tabs.Trigger>
          {/each}
        </Tabs.List>
        <!-- 不具合の報告や、更新できたかの確認に使う。狭いときはページの一番下に出す -->
        <div
          class="mt-auto self-start px-[15px] pt-3 tabular-nums group-data-horizontal/tabs:hidden"
        >
          <AboutDialog />
        </div>
      </div>
      <main bind:this={content} class="min-w-0 flex-1 group-data-vertical/tabs:overflow-y-auto">
        <div class="mx-auto flex {pageWidth} flex-col gap-4 px-4 py-6">
          {#if panel.view?.fallback}
            {@const ownKeys = panel.view.continuesWith === "ownKeys"}
            <Alert.Root>
              <CircleAlertIcon />
              <Alert.Title
                >{ownKeys
                  ? "自分の API キーで続けています"
                  : "Chrome の作り方で続けています"}</Alert.Title
              >
              <Alert.Description>{fallbackText(panel.view.fallback, ownKeys)}</Alert.Description>
            </Alert.Root>
          {/if}
          {#if panel.view?.unsaved}
            <Alert.Root variant="destructive">
              <CircleAlertIcon />
              <Alert.Title>設定を保存できません</Alert.Title>
              <Alert.Description>
                設定ファイルを読めないため、変更は保存されません（{panel.view.unsaved}）
              </Alert.Description>
            </Alert.Root>
          {/if}
          <Onboarding open={(value) => (panel.tab = value)} />
          {#if panel.needsKey && s.onboardingClosed}
            <Alert.Root>
              <KeyRoundIcon />
              <Alert.Title>この作り方には {keyNames(panel.missingKeys)}が要ります</Alert.Title>
              <Alert.Description class="flex flex-wrap gap-2 pt-1">
                <Button size="sm" onclick={() => (panel.tab = "keys")}
                  >{enterKeys(panel.missingKeys)}</Button
                >
                <Button
                  size="sm"
                  variant="outline"
                  onclick={() => panel.update({ engines: CHROME })}
                >
                  無料の Chrome モードにする
                </Button>
              </Alert.Description>
            </Alert.Root>
          {/if}
          {#if s.transcriptionEnabled && panel.transcriptionError}
            <Alert.Root variant="destructive">
              <CircleAlertIcon />
              <Alert.Title>字幕が止まっています</Alert.Title>
              <Alert.Description>
                {transcriptionErrorText(
                  panel.transcriptionError,
                )}。直したら「字幕を止める」→「字幕を始める」で始め直してください
              </Alert.Description>
            </Alert.Root>
          {/if}

          {#each TABS as t (t.value)}
            <Tabs.Content value={t.value}><t.page /></Tabs.Content>
          {/each}
          <div class="hidden tabular-nums group-data-horizontal/tabs:block">
            <AboutDialog />
          </div>
        </div>
      </main>
    </Tabs.Root>
  {/if}
</div>
