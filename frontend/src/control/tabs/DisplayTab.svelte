<script lang="ts">
  import PlayIcon from "@lucide/svelte/icons/play";
  import AlignCenterIcon from "@lucide/svelte/icons/text-align-center";
  import AlignEndIcon from "@lucide/svelte/icons/text-align-end";
  import AlignStartIcon from "@lucide/svelte/icons/text-align-start";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Switch } from "$lib/components/ui/switch";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { Align } from "../../generated/Align";
  import type { Background } from "../../generated/Background";
  import type { SubtitleDisplayPatch } from "../../generated/SubtitleDisplayPatch";
  import CopyUrl from "../parts/CopyUrl.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import DisplayDetailsDialog from "../parts/DisplayDetailsDialog.svelte";
  import { overlayUrl, panel } from "../panel.svelte";

  /** OBS のブラウザソースの大きさ（目安。幅と高さを変えれば中身が合わせる） */
  const SUBTITLE_SIZE: [number, number] = [1000, 400];

  const ALIGNS: { value: Align; label: string; icon: typeof AlignStartIcon }[] = [
    { value: "left", label: "左寄せ", icon: AlignStartIcon },
    { value: "center", label: "中央", icon: AlignCenterIcon },
    { value: "right", label: "右寄せ", icon: AlignEndIcon },
  ];
  /** 話した言葉と翻訳の大きさの組み合わせ。細かく決めたいときは「数値で指定」 */
  const SIZES = [
    { label: "小", fontSize: 32, translationFontSize: 22 },
    { label: "中", fontSize: 40, translationFontSize: 26 },
    { label: "大", fontSize: 48, translationFontSize: 32 },
    { label: "特大", fontSize: 56, translationFontSize: 36 },
  ];
  /** 縁取りの黒に映える、明るい色だけを並べる */
  const COLORS = [
    { hex: "#ffffff", label: "白" },
    { hex: "#fff3c4", label: "クリーム" },
    { hex: "#ffe066", label: "黄" },
    { hex: "#c9f0ff", label: "水色" },
    { hex: "#d4ffc9", label: "薄緑" },
  ];
  const BACKGROUNDS: { value: Background; label: string; hint: string }[] = [
    { value: "outline", label: "縁取り", hint: "文字に黒い縁取りを付けます" },
    {
      value: "band",
      label: "半透明の帯",
      hint: "明るい場面でも読めるよう、行の後ろに黒い帯を敷きます",
    },
  ];
  const LINES = [1, 2, 3, 4, 5];
  const HOLDS = [4, 6, 10, 15];

  const s = $derived(panel.settings!);
  const d = $derived(s.subtitleDisplay);
  /** 変えた項目だけを送る（続けて変えても、前の変更を古い値で戻さない） */
  const set = (patch: SubtitleDisplayPatch) => panel.update({ subtitleDisplay: patch });
  /** 数値で指定して組み合わせから外れたら、どれも選ばない */
  const size = $derived(
    SIZES.find((p) => p.fontSize === d.fontSize && p.translationFontSize === d.translationFontSize)
      ?.label ?? "",
  );
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>字幕オーバーレイ</Card.Title>
      <Card.Description
        >ここで変えた見せ方は、OBS に表示中の字幕にもそのまま反映されます</Card.Description
      >
      <Card.Action class="flex gap-1">
        <DisplayDetailsDialog />
        <Button variant="outline" size="sm" onclick={panel.demo}>
          <PlayIcon />テスト字幕を流す
        </Button>
      </Card.Action>
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
      <OverlayPreview
        src="/overlay/subtitles.html"
        title="字幕オーバーレイのプレビュー"
        size={SUBTITLE_SIZE}
      />

      <Field.Group>
        <Field.Field>
          <Field.Label>揃え</Field.Label>
          <Field.Description
            >OBS で字幕を置く位置に合わせます（画面の左に置くなら左寄せ）</Field.Description
          >
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={d.align}
            onValueChange={(v) => v && set({ align: v as Align })}
          >
            {#each ALIGNS as a (a.value)}
              <ToggleGroup.Item value={a.value} class="flex-1"><a.icon />{a.label}</ToggleGroup.Item
              >
            {/each}
          </ToggleGroup.Root>
        </Field.Field>
        <Field.Field>
          <Field.Label>文字の大きさ</Field.Label>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={size}
            onValueChange={(v) => {
              const preset = SIZES.find((p) => p.label === v);
              if (preset) {
                const { fontSize, translationFontSize } = preset;
                void set({ fontSize, translationFontSize });
              }
            }}
          >
            {#each SIZES as p (p.label)}
              <ToggleGroup.Item value={p.label} class="flex-1">{p.label}</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
          {#if !size}
            <Field.Description>
              数値で指定しています（話した言葉 {d.fontSize}px・翻訳 {d.translationFontSize}px）
            </Field.Description>
          {/if}
        </Field.Field>
        <Field.Field>
          <Field.Label>文字の色</Field.Label>
          <Field.Description>
            確定した字幕の色です。ヒーロー名とスキル名の色は、上の「詳しく設定」から変えられます（アイテム名はスロットの色のまま）
          </Field.Description>
          <div class="flex flex-wrap items-center gap-2">
            {#each COLORS as c (c.hex)}
              <button
                type="button"
                class="size-8 rounded-full border-2 ring-offset-2 ring-offset-background transition {d.textColor ===
                c.hex
                  ? 'border-primary ring-2 ring-primary'
                  : 'border-border'}"
                style:background={c.hex}
                aria-label={c.label}
                aria-pressed={d.textColor === c.hex}
                onclick={() => set({ textColor: c.hex })}
              ></button>
            {/each}
            <!-- 並べた色以外は、OS の色選びから -->
            <label class="flex items-center gap-2 text-sm text-muted-foreground">
              <input
                type="color"
                class="size-8 cursor-pointer rounded-full border-0 bg-transparent p-0"
                value={d.textColor}
                onchange={(e) => set({ textColor: e.currentTarget.value })}
              />
              ほかの色
            </label>
          </div>
        </Field.Field>
        <Field.Field>
          <Field.Label>背景</Field.Label>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={d.background}
            onValueChange={(v) => v && set({ background: v as Background })}
          >
            {#each BACKGROUNDS as b (b.value)}
              <ToggleGroup.Item value={b.value} class="flex-1">{b.label}</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
          <Field.Description
            >{BACKGROUNDS.find((b) => b.value === d.background)?.hint}</Field.Description
          >
        </Field.Field>

        <Field.Separator />

        <Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="furigana">ふりがなを付ける</Field.Label>
            <Field.Description
              >漢字に読みを付けます（日本語を学んでいる視聴者向け）</Field.Description
            >
          </Field.Content>
          <Switch
            id="furigana"
            checked={d.furigana}
            onCheckedChange={(v) => set({ furigana: v })}
          />
        </Field.Field>

        <Field.Separator />

        <Field.Field>
          <Field.Label>同時に出す字幕の数</Field.Label>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={String(d.maxLines)}
            onValueChange={(v) => v && set({ maxLines: Number(v) })}
          >
            {#each LINES as n (n)}
              <ToggleGroup.Item value={String(n)} class="flex-1">{n}</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
        </Field.Field>
        <Field.Field>
          <Field.Label>消すまでの時間</Field.Label>
          <Field.Description>長い文は、そのぶん長く出します</Field.Description>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={String(d.holdSeconds)}
            onValueChange={(v) => v && set({ holdSeconds: Number(v) })}
          >
            {#each HOLDS as n (n)}
              <ToggleGroup.Item value={String(n)} class="flex-1">{n} 秒</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
        </Field.Field>
      </Field.Group>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>OBS に追加する</Card.Title>
      <Card.Description>
        ソースの「＋」→「ブラウザ」で、この URL を貼り付け、幅 {SUBTITLE_SIZE[0]}・高さ
        {SUBTITLE_SIZE[1]} にして好きな位置に置いてください。字幕は箱の下から積み上がり、箱の幅で折り返します。
        大きさを変えるときは、ソースを引き伸ばさずにプロパティの幅と高さを変えてください（引き伸ばすと文字ごと伸びます）。
        一度追加すれば、見せ方を変えても貼り直す必要はありません
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <CopyUrl url={overlayUrl("subtitles")} />
    </Card.Content>
  </Card.Root>
</div>
