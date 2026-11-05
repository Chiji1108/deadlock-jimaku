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
  import { toast } from "svelte-sonner";
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

  const s = $derived(panel.settings!);
  const onlyTranslation = $derived(translationOnly(s));
  const chrome = $derived(s.speechEngine === "chrome");

  async function openChrome() {
    if (!(await api.openChrome().catch(() => false)))
      toast.error("Chrome を開けませんでした。Google Chrome を入れてから、もう一度押してください");
  }
  const pause = $derived(s.segmentation.minSilenceMs);
  const preset = $derived(PAUSES.find((p) => p.ms === pause));

  const DEFAULT_DEVICE = "__default__";
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>音声認識と翻訳</Card.Title>
    </Card.Header>
    <Card.Content class="flex flex-col gap-4">
      <ToggleGroup.Root
        type="single"
        variant="outline"
        class="w-full"
        value={s.speechEngine}
        onValueChange={(v) => v && panel.update({ speechEngine: v as SpeechEngine })}
      >
        <ToggleGroup.Item value="openai" class="flex-1">OpenAI</ToggleGroup.Item>
        <ToggleGroup.Item value="chrome" class="flex-1">Chrome（無料）</ToggleGroup.Item>
      </ToggleGroup.Root>
      {#if chrome}
        <p class="text-sm text-muted-foreground">
          Google Chrome の中で音声認識と翻訳を動かします。API キーは要りません。音声は PC
          の外に送られません。精度は下がり、用語の強調、ふりがな、アイテム説明は使えません。下のボタンで開いた
          Chrome を閉じずに、「字幕を始める」を押してください
        </p>
        <Button variant="outline" onclick={openChrome}>Chrome を開く</Button>
      {:else}
        <p class="text-sm text-muted-foreground">
          精度が高く、用語の強調とふりがなが使えます。API キーかギフトコードが必要です
        </p>
      {/if}
    </Card.Content>
  </Card.Root>

  {#if !chrome}<Card.Root>
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
      </Card.Content>
    </Card.Root>{/if}

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
      </Field.Field>

      {#if !chrome}<Field.Field orientation="horizontal">
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

        <Field.Field orientation="horizontal">
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
          最大 {MAX_TRANSLATE_TO} つ（{s.translateTo.length} / {MAX_TRANSLATE_TO}）。1
          つ増えるごとに、話し続けて 1 時間あたり約 $0.3
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
