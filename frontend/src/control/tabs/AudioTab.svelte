<script lang="ts">
  import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import LevelMeter from "../parts/LevelMeter.svelte";
  import { panel } from "../panel.svelte";

  const MAX_TRANSLATE_TO = 4;
  /** 何 ms 黙ったら区切るか */
  const PAUSES = [
    { ms: 300, label: "早め", hint: "すぐ字幕になる。文の途中で切れやすい" },
    { ms: 400, label: "ふつう", hint: "おすすめ" },
    { ms: 700, label: "ゆったり", hint: "文がまとまる。出るまで少し待つ" },
  ];

  const s = $derived(panel.settings!);
  const pause = $derived(s.segmentation.minSilenceMs);
  const preset = $derived(PAUSES.find((p) => p.ms === pause));
  let custom = $state(false);

  const DEFAULT_DEVICE = "__default__";
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
              {#each panel.devices as name (name)}
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
        {#if panel.devices.length === 0}
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
            {#each panel.languages as l (l.code)}
              <Select.Item value={l.code}>{l.label}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
      </Field.Field>

      <Field.Field>
        <Field.Label>翻訳先</Field.Label>
        <Field.Description>
          最大 {MAX_TRANSLATE_TO} つ（{s.translateTo.length} / {MAX_TRANSLATE_TO}）。1
          つ増えるごとに 1 時間あたり約 $0.3
        </Field.Description>
        <ToggleGroup.Root
          type="multiple"
          variant="outline"
          spacing={2}
          class="flex-wrap justify-start"
          value={s.translateTo}
          onValueChange={(v) => panel.update({ translateTo: v })}
        >
          {#each panel.languages.filter((l) => l.code !== s.micLanguage) as l (l.code)}
            <ToggleGroup.Item
              value={l.code}
              size="sm"
              disabled={!s.translateTo.includes(l.code) && s.translateTo.length >= MAX_TRANSLATE_TO}
            >
              {l.label}
            </ToggleGroup.Item>
          {/each}
        </ToggleGroup.Root>
        {#if s.translateTo.length > 0 && !panel.view?.keys.openai}
          <Field.Description class="text-amber-500">
            翻訳するには「API キー」で OpenAI のキーを設定してください
          </Field.Description>
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
    </Card.Header>
    <Card.Content class="flex flex-col gap-3">
      <ToggleGroup.Root
        type="single"
        variant="outline"
        class="w-full"
        value={custom || !preset ? "custom" : String(pause)}
        onValueChange={(v) => {
          if (!v) return;
          custom = v === "custom";
          if (!custom) void panel.update({ minSilenceMs: Number(v) });
        }}
      >
        {#each PAUSES as p (p.ms)}
          <ToggleGroup.Item value={String(p.ms)} class="flex-1 flex-col gap-0 py-6">
            <span>{p.label}</span>
            <span class="text-xs text-muted-foreground">{p.ms / 1000} 秒</span>
          </ToggleGroup.Item>
        {/each}
        <ToggleGroup.Item value="custom" class="flex-1 flex-col gap-0 py-6">
          <span>細かく</span>
          <span class="text-xs text-muted-foreground">自分で決める</span>
        </ToggleGroup.Item>
      </ToggleGroup.Root>
      {#if custom || !preset}
        <Field.Field orientation="horizontal">
          <Input
            type="number"
            class="w-32"
            min={200}
            max={2000}
            step={50}
            value={pause}
            onchange={(e) => panel.update({ minSilenceMs: e.currentTarget.valueAsNumber })}
          />
          <Field.Label>ミリ秒（200〜2000）</Field.Label>
        </Field.Field>
      {:else}
        <p class="text-sm text-muted-foreground">{preset.hint}</p>
      {/if}
    </Card.Content>
  </Card.Root>
</div>
