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
  import { panel } from "../panel.svelte";
  import { chromeFinals, llmTranslation } from "../../lib/engines";

  const MAX_TRANSLATE_TO = 4;
  /** 何 ms 黙ったら区切るか */
  const PAUSES = [
    { ms: 500, label: "すぐ", hint: "早く確定する。文の途中で切れやすい" },
    { ms: 750, label: "ふつう", hint: "おすすめ" },
    { ms: 1000, label: "つなぐ", hint: "文がまとまる。確定まで少し待つ" },
  ];

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const pause = $derived(s.segmentation.minSilenceMs);
  const preset = $derived(PAUSES.find((p) => p.ms === pause));

  const DEFAULT_DEVICE = "__default__";
  /** 翻訳先は、選んだ言語だけを並べておく。ほかの言語は、開いたときに出す（1 つも選んでいなければ、開いておく） */
  let allTargets = $state(false);
  const targets = $derived(
    panel.languages.filter(
      (code) =>
        code !== s.micLanguage &&
        (allTargets || s.translateTo.length === 0 || s.translateTo.includes(code)),
    ),
  );
  const moreTargets = $derived(
    panel.languages.filter((code) => code !== s.micLanguage).length - targets.length,
  );
</script>

<div class="flex flex-col gap-6">
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
          最大 {MAX_TRANSLATE_TO} つ（{s.translateTo.length} / {MAX_TRANSLATE_TO}）{#if llmTranslation(t)}。1
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
          {#each targets as code (code)}
            <ToggleGroup.Item
              value={code}
              size="sm"
              disabled={!s.translateTo.includes(code) && s.translateTo.length >= MAX_TRANSLATE_TO}
            >
              {panel.label(code)}
            </ToggleGroup.Item>
          {/each}
        </ToggleGroup.Root>
        {#if moreTargets > 0 || (allTargets && s.translateTo.length > 0)}
          <Button
            variant="link"
            size="sm"
            class="h-auto w-fit p-0 text-xs text-muted-foreground"
            onclick={() => (allTargets = !allTargets)}
          >
            {allTargets ? "選んだ言語だけにする" : `ほかの言語を選ぶ（${moreTargets}）`}
          </Button>
        {/if}
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
