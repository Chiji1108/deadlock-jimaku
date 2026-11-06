<script lang="ts">
  import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import * as Select from "$lib/components/ui/select";
  import { Switch } from "$lib/components/ui/switch";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import DeadlockModeDialog from "../parts/DeadlockModeDialog.svelte";
  import LevelMeter from "../parts/LevelMeter.svelte";
  import PauseDialog from "../parts/PauseDialog.svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { api } from "../../lib/api";
  import { panel } from "../panel.svelte";
  import { translationOnly } from "../../lib/subtitle-lines";
  import type { SpeechEngine } from "../../generated/SpeechEngine";

  const MAX_TRANSLATE_TO = 4;
  /** 何 ms 黙ったら区切るか */
  const PAUSES = [
    { ms: 500, label: "すぐ", hint: "早く確定する。文の途中で切れやすい" },
    { ms: 750, label: "ふつう", hint: "おすすめ" },
    { ms: 1000, label: "つなぐ", hint: "文がまとまる。確定まで少し待つ" },
  ];

  const ENGINES: {
    value: SpeechEngine;
    label: string;
    traits: string;
    price: string;
    hint: string;
    recommended?: boolean;
  }[] = [
    {
      value: "hybrid",
      label: "ハイブリッド",
      traits: "高速・高精度",
      price: "約 $0.6／時間",
      hint: "途中の字幕は PC で作り、確定の字幕は OpenAI の高い精度に置き換えます",
      recommended: true,
    },
    {
      value: "openai",
      label: "OpenAI",
      traits: "低負荷・高精度",
      price: "約 $1／時間",
      hint: "すべて OpenAI で作ります。PC の負荷は最小です",
    },
    {
      value: "local",
      label: "ローカルAI",
      traits: "無料・高速",
      price: "無料",
      hint: "PC だけで作ります。無料でキー不要、精度は低めです",
    },
  ];

  const s = $derived(panel.settings!);
  const onlyTranslation = $derived(translationOnly(s));
  const engine = $derived(ENGINES.find((e) => e.value === s.speechEngine)!);
  /** ローカルAI が止まっている理由 */
  const localProblem = $derived(
    panel.liveStatus?.state === "failed" ? panel.liveStatus.detail : undefined,
  );
  const pause = $derived(s.segmentation.minSilenceMs);
  const preset = $derived(PAUSES.find((p) => p.ms === pause));

  const DEFAULT_DEVICE = "__default__";
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>モード</Card.Title>
      <Card.Description>字幕をどこで作るかを選びます</Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-3">
      <div class="grid gap-2 sm:grid-cols-3" role="radiogroup" aria-label="モード">
        {#each ENGINES as e (e.value)}
          <button
            type="button"
            role="radio"
            aria-checked={s.speechEngine === e.value}
            class="flex flex-col gap-1 rounded-lg border p-3 text-left transition-colors hover:bg-muted aria-checked:border-foreground aria-checked:bg-muted"
            onclick={() => panel.update({ speechEngine: e.value })}
          >
            <span class="flex items-center gap-2 text-sm font-medium">
              {e.label}
              {#if e.recommended}<Badge variant="secondary">おすすめ</Badge>{/if}
            </span>
            <span class="text-xs text-muted-foreground">{e.traits}</span>
            <span class="text-xs text-muted-foreground tabular-nums">{e.price}</span>
          </button>
        {/each}
      </div>
      <p class="text-sm text-muted-foreground">{engine.hint}</p>
      <p class="text-xs text-muted-foreground">
        ハイブリッドとローカルAIは、初回に約 200MB をダウンロードします
      </p>
      {#if s.speechEngine !== "openai" && s.transcriptionEnabled && panel.liveStatus?.state === "preparing"}
        <p class="text-sm">{panel.liveStatus.detail ?? "モデルを読み込んでいます…"}</p>
      {/if}
      {#if s.speechEngine !== "openai" && s.transcriptionEnabled && localProblem}
        <Alert.Root variant="destructive">
          <Alert.Title>{localProblem}</Alert.Title>
          {#if s.speechEngine === "hybrid"}
            <Alert.Description>
              確定の字幕は出ています。話している途中の字幕だけが止まっています。
            </Alert.Description>
          {/if}
        </Alert.Root>
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

      <Field.Field>
        <Field.Label>入力レベル</Field.Label>
        <LevelMeter />
        {#if !panel.running}
          <Field.Description>字幕を始めると、マイクに届いている音の大きさが出ます</Field.Description
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
        {#if s.speechEngine === "hybrid" && s.micLanguage !== "ja"}
          <Field.Description>途中の字幕は日本語のときだけ出ます</Field.Description>
        {/if}
      </Field.Field>

      <Field.Field orientation="horizontal">
        <Field.Content>
          <Field.Label for="deadlock-mode">デッドロックモード</Field.Label>
          <Field.Description>
            ヒーロー・スキル・アイテムを強調し、アイテム説明を出します。雑談やほかのゲームの配信では、オフにすると普通の字幕になります
            <DeadlockModeDialog />
          </Field.Description>
        </Field.Content>
        <Switch
          id="deadlock-mode"
          checked={s.deadlockMode}
          onCheckedChange={(v) => panel.update({ deadlockMode: v })}
        />
      </Field.Field>

      {#if s.speechEngine === "openai"}<Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="live-subtitles">話している途中から字幕を出す</Field.Label>
            <Field.Description>
              {#if onlyTranslation}
                翻訳だけを出しているので、途中の字幕は出ません
              {:else}
                オフにすると途中の字幕の料金がかかりません。字幕は話し終えてから出ます。切り替えると、字幕は始め直します
              {/if}
            </Field.Description>
          </Field.Content>
          <Switch
            id="live-subtitles"
            checked={s.liveSubtitles}
            disabled={onlyTranslation}
            onCheckedChange={(v) => panel.update({ liveSubtitles: v })}
          />
        </Field.Field>{/if}

      <Field.Field>
        <Field.Label>翻訳先</Field.Label>
        <Field.Description>
          最大 {MAX_TRANSLATE_TO} つ（{s.translateTo.length} / {MAX_TRANSLATE_TO}）{#if s.speechEngine === "local"}。ローカルAIで訳せるのは英語・ロシア語・中国語です{:else}。1
            つ増えるごとに、話し続けて 1 時間あたり約 $0.3{/if}
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
