<script lang="ts">
  import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import * as Select from "$lib/components/ui/select";
  import { Switch } from "$lib/components/ui/switch";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import ConfirmDialog from "../parts/ConfirmDialog.svelte";
  import DeadlockModeDialog from "../parts/DeadlockModeDialog.svelte";
  import LevelMeter from "../parts/LevelMeter.svelte";
  import PauseDialog from "../parts/PauseDialog.svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { panel } from "../panel.svelte";
  import EnginesDialog from "../parts/EnginesDialog.svelte";
  import {
    STANDARD,
    MODES,
    monthly,
    price,
    anyPaid,
    chromeFinals,
    modeOf,
    keyNames,
    paidItems,
    providers,
    usesChrome,
  } from "../../lib/engines";

  const MAX_TRANSLATE_TO = 4;
  /** 何 ms 黙ったら区切るか */
  const PAUSES = [
    { ms: 500, label: "すぐ", hint: "早く確定する。文の途中で切れやすい" },
    { ms: 750, label: "ふつう", hint: "おすすめ" },
    { ms: 1000, label: "つなぐ", hint: "文がまとまる。確定まで少し待つ" },
  ];

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const mode = $derived(modeOf(t));
  const paid = $derived(paidItems(t));
  /** Chrome のウィンドウが字幕を止めている理由。止まっているか、回線の回復を待っている */
  const chromeProblem = $derived(
    panel.chromeStatus?.state === "failed" ||
      panel.chromeStatus?.state === "blocked" ||
      panel.chromeStatus?.state === "missing"
      ? panel.chromeStatus.detail
      : undefined,
  );
  /** 有料の工程がある字幕が止まったら Chrome を閉じる（字幕にならない音声を Google に送らない） */
  const chromeClosed = $derived(anyPaid(t) && !!panel.transcriptionError);
  /** Chrome のウィンドウの今の状態 */
  const chromeNote = $derived.by(() => {
    if (chromeClosed) return "字幕が止まっているので閉じています";
    if (!panel.chromeStatus) return "Chrome のウィンドウを開いています";
    if (panel.chromeStatus.detail) return panel.chromeStatus.detail;
    if (panel.chromeStatus.state === "listening") return "聞き取っています";
    return "字幕の準備をしています";
  });
  const pause = $derived(s.segmentation.minSilenceMs);
  const preset = $derived(PAUSES.find((p) => p.ms === pause));

  const DEFAULT_DEVICE = "__default__";
  let confirmingReset = $state(false);
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>モード</Card.Title>
      <Card.Description>字幕をどこで作るかを選びます</Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-3">
      <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-4" role="radiogroup" aria-label="モード">
        {#each MODES as m (m.name)}
          <button
            type="button"
            role="radio"
            aria-checked={mode?.name === m.name}
            class="flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors hover:bg-muted aria-checked:border-foreground aria-checked:bg-muted"
            onclick={() => panel.update({ engines: m.engines })}
          >
            <span class="flex items-center gap-2 text-sm font-medium">
              {m.label}
              {#if m.recommended}<Badge variant="secondary">おすすめ</Badge>{/if}
            </span>
            <span class="text-xs text-muted-foreground">{m.traits}</span>
            <span class="text-xs text-muted-foreground tabular-nums">{price(m.hourlyUsd)}</span>
            {#if m.hourlyUsd}
              <span class="text-xs text-muted-foreground tabular-nums">{monthly(m.hourlyUsd)}</span>
            {/if}
          </button>
        {/each}
      </div>
      <p class="text-sm text-muted-foreground">
        {#if mode}
          {mode.hint}
        {:else if paid.length}
          詳しく選んだ作り方です。有料の工程: {paid.join("、")}
        {:else}
          詳しく選んだ作り方です。すべて無料です
        {/if}{#if paid.length}。{keyNames(providers(t))}が要ります{/if}
      </p>
      <EnginesDialog />
      {#if mode?.name === "chrome" && panel.standardReady && !s.hybridHintClosed}
        <div class="flex flex-wrap items-center justify-between gap-2 rounded-lg border p-3">
          <p class="text-sm">スタンダードにすると、確定の字幕が高い精度になります</p>
          <div class="flex gap-1">
            <Button variant="outline" size="sm" onclick={() => panel.update({ engines: STANDARD })}>
              スタンダードにする
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onclick={() => panel.update({ hybridHintClosed: true })}
            >
              閉じる
            </Button>
          </div>
        </div>
      {/if}
      {#if usesChrome(t)}
        {#if s.transcriptionEnabled}
          <div class="flex flex-wrap items-center gap-3">
            {#if !chromeClosed}
              <Button variant="outline" size="sm" onclick={panel.openChrome}
                >Chrome を開き直す</Button
              >
            {/if}
            {#if !chromeProblem}<p class="text-sm text-muted-foreground">{chromeNote}</p>{/if}
          </div>
        {/if}
        <p class="text-xs text-muted-foreground">
          字幕を始めると小さな Chrome のウィンドウが開きます。音声は Chrome を通じて Google
          に送られます。途中の翻訳は Chrome
          の中で動きます。字幕を出す間はウィンドウを閉じないでください
        </p>
        {#if s.transcriptionEnabled && chromeProblem}
          <Alert.Root variant="destructive">
            <Alert.Title>{chromeProblem}</Alert.Title>
            <Alert.Description class="flex flex-col items-start gap-2">
              {#if !chromeFinals(t)}
                確定の字幕は出ています。話している途中の字幕だけが止まっています。
              {/if}
              {#if panel.chromeStatus?.state === "missing"}
                <Button
                  variant="outline"
                  size="sm"
                  href="https://www.google.com/chrome/"
                  target="_blank"
                  rel="noreferrer"
                >
                  Google Chrome をインストールする
                </Button>
              {:else}
                {@render resetButton()}
              {/if}
            </Alert.Description>
          </Alert.Root>
        {:else}
          <div class="flex flex-wrap items-center gap-3">
            <p class="text-xs text-muted-foreground">
              マイクの許可をやり直したいとき、字幕が出ないときに
            </p>
            {@render resetButton()}
          </div>
        {/if}
      {/if}
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>マイク</Card.Title>
      <Card.Description>配信に乗せている声のマイクを選んでください</Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-5">
      <Field.Field>
        <Field.Label>入力デバイス</Field.Label>
        <div class="flex gap-2">
          <Select.Root
            type="single"
            value={s.micDevice ?? DEFAULT_DEVICE}
            onValueChange={(v) => panel.update({ micDevice: v === DEFAULT_DEVICE ? "" : v })}
            onOpenChange={(open) => open && void panel.refreshDevices()}
          >
            <Select.Trigger class="flex-1">{s.micDevice ?? "OS の既定のマイク"}</Select.Trigger>
            <Select.Content>
              <Select.Item value={DEFAULT_DEVICE}>OS の既定のマイク</Select.Item>
              {#each panel.devices ?? [] as name (name)}
                <Select.Item value={name}>{name}</Select.Item>
              {/each}
            </Select.Content>
          </Select.Root>
          <Button
            variant="outline"
            size="icon"
            aria-label="デバイスの一覧を更新"
            onclick={() => panel.refreshDevices()}
          >
            <RefreshCwIcon />
          </Button>
        </div>
        {#if panel.devices?.length === 0}
          <Field.Description class="text-amber-500">
            マイクが見つかりません。つないだら更新ボタンを押してください
          </Field.Description>
        {/if}
      </Field.Field>

      {#if !chromeFinals(t)}
        <Field.Field>
          <Field.Label>入力レベル</Field.Label>
          <LevelMeter />
          {#if !panel.running}
            <Field.Description
              >字幕を始めると、マイクに届いている音の大きさが出ます</Field.Description
            >
          {/if}
        </Field.Field>

        {#if panel.silent}
          <Alert.Root variant="destructive">
            <Alert.Title>音が届いていません</Alert.Title>
            <Alert.Description>
              マイクの選択と、OS のマイクの許可（macOS: システム設定 → プライバシーとセキュリティ →
              マイク）を確認してください
            </Alert.Description>
          </Alert.Root>
        {/if}
      {/if}
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>言語</Card.Title>
    </Card.Header>
    <Card.Content class="flex flex-col gap-5">
      <Field.Field>
        <Field.Label>話す言語</Field.Label>
        <Select.Root
          type="single"
          value={s.micLanguage}
          onValueChange={(v) => panel.update({ micLanguage: v })}
        >
          <Select.Trigger class="w-full">{panel.label(s.micLanguage)}</Select.Trigger>
          <Select.Content>
            {#each panel.languages as code (code)}
              <Select.Item value={code}>{panel.label(code)}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
        {#if t.translation === "chromeTranslator" && s.micLanguage !== "ja"}
          <Field.Description>ふりがなと用語の強調は日本語のときだけ出ます</Field.Description>
        {/if}
      </Field.Field>

      <Field.Field orientation="horizontal">
        <Field.Content>
          <Field.Label for="deadlock-mode">デッドロックモード</Field.Label>
          <Field.Description>
            ヒーロー・スキル・アイテムを強調し、アイテム説明を出します
            <DeadlockModeDialog />
          </Field.Description>
        </Field.Content>
        <Switch
          id="deadlock-mode"
          checked={s.deadlockMode}
          onCheckedChange={(v) => panel.update({ deadlockMode: v })}
        />
      </Field.Field>

      <Field.Field>
        <Field.Label>翻訳先</Field.Label>
        <Field.Description>
          最大 {MAX_TRANSLATE_TO} つ（{s.translateTo.length} / {MAX_TRANSLATE_TO}）{#if t.translation !== "chromeTranslator"}。1
            つ増えるごとに、話し続けて 1 時間あたり約 ${t.translation === "gpt6Luna"
              ? "0.05"
              : "0.3"}{/if}
        </Field.Description>
        <ToggleGroup.Root
          type="multiple"
          variant="outline"
          spacing={2}
          class="flex-wrap justify-start"
          value={s.translateTo}
          onValueChange={(v) => panel.update({ translateTo: v })}
        >
          {#each panel.languages.filter((code) => code !== s.micLanguage) as code (code)}
            <ToggleGroup.Item
              value={code}
              size="sm"
              disabled={!s.translateTo.includes(code) && s.translateTo.length >= MAX_TRANSLATE_TO}
            >
              {panel.label(code)}
            </ToggleGroup.Item>
          {/each}
        </ToggleGroup.Root>
      </Field.Field>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>文の区切り</Card.Title>
      <Card.Description
        >話し終えてから、どれだけ黙ったら 1 つの字幕として確定するか</Card.Description
      >
      <Card.Action><PauseDialog /></Card.Action>
    </Card.Header>
    <Card.Content class="flex flex-col gap-3">
      <ToggleGroup.Root
        type="single"
        variant="outline"
        class="w-full"
        value={preset ? String(pause) : ""}
        onValueChange={(v) => v && panel.update({ minSilenceMs: Number(v) })}
      >
        {#each PAUSES as p (p.ms)}
          <ToggleGroup.Item value={String(p.ms)} class="flex-1 flex-col gap-0 py-6">
            <span>{p.label}</span>
            <span class="text-xs text-muted-foreground">{p.ms / 1000} 秒</span>
          </ToggleGroup.Item>
        {/each}
      </ToggleGroup.Root>
      <p class="text-sm text-muted-foreground">
        {preset ? preset.hint : `数値で指定しています（${pause} ミリ秒）`}
      </p>
    </Card.Content>
  </Card.Root>
</div>

{#snippet resetButton()}
  <Button variant="outline" size="sm" onclick={() => (confirmingReset = true)}>
    Chrome をリセット
  </Button>
{/snippet}

<ConfirmDialog
  bind:open={confirmingReset}
  title="Chrome をリセットしますか？"
  action="リセットする"
  destructive
  onconfirm={panel.resetChrome}
>
  <p>
    字幕に使う Chrome のウィンドウを閉じて、専用のデータを消します。ふだん使っている Chrome
    には影響しません。
  </p>
  <p>
    マイクの許可はもう一度聞かれ、ウィンドウで選んだマイクは戻ります。翻訳のモデルもダウンロードし直すことがあります。
  </p>
  {#if s.transcriptionEnabled}<p>字幕を出している間なら、Chrome は自動で起動し直します。</p>{/if}
</ConfirmDialog>
