<script lang="ts">
  import PlayIcon from "@lucide/svelte/icons/play";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import * as Select from "$lib/components/ui/select";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import CopyUrl from "../parts/CopyUrl.svelte";
  import ItemHoldDialog from "../parts/ItemHoldDialog.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { overlayUrl, panel } from "../panel.svelte";

  /** OBS のブラウザソースの大きさ（カードは幅 450。長い説明でも収まる高さ） */
  const ITEMS_SIZE: [number, number] = [460, 560];
  const HOLDS = [5, 8, 12, 20];
  const FOLLOW = "";

  const s = $derived(panel.settings!);
  const c = $derived(s.itemCards);
  const language = $derived(c.language ?? FOLLOW);
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>アイテム説明オーバーレイ</Card.Title>
      <Card.Description>
        字幕でアイテム名が出たら、そのアイテムの説明をゲーム内と同じカードで出します
      </Card.Description>
      <Card.Action>
        <Button variant="outline" size="sm" onclick={panel.demo}>
          <PlayIcon />テスト字幕を流す
        </Button>
      </Card.Action>
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
      <div class="mx-auto w-full max-w-xs">
        <OverlayPreview
          src="/overlay/items.html"
          title="アイテム説明オーバーレイのプレビュー"
          size={ITEMS_SIZE}
        />
      </div>

      <Field.Group>
        <Field.Field>
          <div class="flex items-center justify-between">
            <Field.Label>出しておく時間</Field.Label>
            <ItemHoldDialog />
          </div>
          <Field.Description>
            続けて話したアイテムは順番に 1
            枚ずつ出します。出しているアイテムをもう一度話すと、最初から数え直します
          </Field.Description>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={String(c.holdSeconds)}
            onValueChange={(v) => v && panel.update({ itemCards: { holdSeconds: Number(v) } })}
          >
            {#each HOLDS as n (n)}
              <ToggleGroup.Item value={String(n)} class="flex-1">{n} 秒</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
          {#if !HOLDS.includes(c.holdSeconds)}
            <Field.Description>数値で指定しています（{c.holdSeconds} 秒）</Field.Description>
          {/if}
        </Field.Field>
        <Field.Field>
          <Field.Label>説明の言語</Field.Label>
          <Select.Root
            type="single"
            value={language}
            onValueChange={(v) => panel.update({ itemCards: { language: v } })}
          >
            <Select.Trigger class="w-full">
              {language === FOLLOW
                ? `話す言語に合わせる（${panel.label(s.micLanguage)}）`
                : panel.label(language)}
            </Select.Trigger>
            <Select.Content>
              <Select.Item value={FOLLOW}>
                話す言語に合わせる（{panel.label(s.micLanguage)}）
              </Select.Item>
              {#each panel.languages as code (code)}
                <Select.Item value={code}>{panel.label(code)}</Select.Item>
              {/each}
            </Select.Content>
          </Select.Root>
          <Field.Description>カードが対応していない言語は英語で出します</Field.Description>
        </Field.Field>
      </Field.Group>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>OBS に追加する</Card.Title>
      <Card.Description>
        ソースの「＋」→「ブラウザ」で、この URL を貼り付け、幅 {ITEMS_SIZE[0]}・高さ
        {ITEMS_SIZE[1]} にして好きな位置に置いてください。カードは箱の左上から出ます。大きさを変えるときは、ソースを引き伸ばさずにプロパティの幅と高さを同じ割合で変えてください。カードが幅に合わせてくっきりしたまま大きくなります
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <CopyUrl url={overlayUrl("items")} />
    </Card.Content>
  </Card.Root>
</div>
