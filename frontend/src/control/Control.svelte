<script lang="ts">
  import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
  import MicIcon from "@lucide/svelte/icons/mic";
  import SquareIcon from "@lucide/svelte/icons/square";
  import { ModeWatcher } from "mode-watcher";
  import { onMount } from "svelte";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import { Toaster } from "$lib/components/ui/sonner";
  import * as Tabs from "$lib/components/ui/tabs";
  import { panel } from "./panel.svelte";
  import AudioTab from "./tabs/AudioTab.svelte";
  import DisplayTab from "./tabs/DisplayTab.svelte";
  import ItemsTab from "./tabs/ItemsTab.svelte";
  import KeysTab from "./tabs/KeysTab.svelte";
  import RankTab from "./tabs/RankTab.svelte";
  import UsageTab from "./tabs/UsageTab.svelte";

  onMount(() => panel.start());

  const s = $derived(panel.settings);
  const needsKey = $derived(!!panel.view && !panel.view.keys.openai);
  let tab = $state("audio");

  const status = $derived.by(() => {
    if (!s?.transcriptionEnabled) return { label: "停止中", dot: "bg-muted-foreground/50" };
    if (panel.transcriptionError) return { label: "止まっています", dot: "bg-destructive" };
    if (panel.silent) return { label: "音が届いていません", dot: "bg-amber-500" };
    if (panel.level === undefined)
      return { label: "準備しています…", dot: "bg-sky-500 animate-pulse" };
    return { label: "字幕を出しています", dot: "bg-emerald-500 animate-pulse" };
  });
</script>

<ModeWatcher />
<Toaster richColors position="bottom-right" />

<div class="min-h-screen bg-background text-foreground">
  <header class="sticky top-0 z-10 border-b bg-background/80 backdrop-blur">
    <div class="mx-auto flex max-w-3xl items-center gap-4 px-4 py-3">
      <div class="flex min-w-0 flex-1 flex-col">
        <h1 class="text-sm font-semibold">Deadlock Stream Overlays</h1>
        <p class="flex items-center gap-2 text-xs text-muted-foreground">
          <span class="size-2 shrink-0 rounded-full {status.dot}"></span>
          {panel.server === "open" ? status.label : "アプリに接続しています…"}
        </p>
      </div>
      {#if s}
        {#if s.transcriptionEnabled}
          <Button variant="outline" onclick={() => panel.setTranscription(false)}>
            <SquareIcon />字幕を止める
          </Button>
        {:else}
          <Button disabled={needsKey} onclick={() => panel.setTranscription(true)}>
            <MicIcon />字幕を始める
          </Button>
        {/if}
      {/if}
    </div>
  </header>

  <main class="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6">
    {#if panel.view?.unsaved}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>設定を保存できません</Alert.Title>
        <Alert.Description>{panel.view.unsaved}</Alert.Description>
      </Alert.Root>
    {/if}
    {#if needsKey}
      <Alert.Root>
        <CircleAlertIcon />
        <Alert.Title>はじめに OpenAI の API キーを設定してください</Alert.Title>
        <Alert.Description>
          <p>
            文字起こしと翻訳に使います。<button class="underline" onclick={() => (tab = "keys")}>
              API キーの設定へ
            </button>
          </p>
        </Alert.Description>
      </Alert.Root>
    {/if}
    {#if s?.transcriptionEnabled && panel.transcriptionError}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>字幕が止まっています</Alert.Title>
        <Alert.Description>
          {panel.transcriptionError}。直したら「字幕を止める」→「字幕を始める」で始め直してください
        </Alert.Description>
      </Alert.Root>
    {/if}

    {#if s}
      <Tabs.Root bind:value={tab}>
        <Tabs.List class="w-full">
          <Tabs.Trigger value="audio">マイクと翻訳</Tabs.Trigger>
          <Tabs.Trigger value="display">字幕の表示</Tabs.Trigger>
          <Tabs.Trigger value="items">アイテム説明</Tabs.Trigger>
          <Tabs.Trigger value="rank">ランク</Tabs.Trigger>
          <Tabs.Trigger value="keys">API キー</Tabs.Trigger>
          <Tabs.Trigger value="usage">料金</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="audio" class="pt-4"><AudioTab /></Tabs.Content>
        <Tabs.Content value="display" class="pt-4"><DisplayTab /></Tabs.Content>
        <Tabs.Content value="items" class="pt-4"><ItemsTab /></Tabs.Content>
        <Tabs.Content value="rank" class="pt-4"><RankTab /></Tabs.Content>
        <Tabs.Content value="keys" class="pt-4"><KeysTab /></Tabs.Content>
        <Tabs.Content value="usage" class="pt-4"><UsageTab /></Tabs.Content>
      </Tabs.Root>
    {/if}
  </main>
</div>
