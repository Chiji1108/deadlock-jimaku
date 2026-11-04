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
  import AddToObs from "../parts/AddToObs.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import DisplayTimingDialog from "../parts/DisplayTimingDialog.svelte";
  import FontSizeDialog from "../parts/FontSizeDialog.svelte";
  import { SUBTITLE_SIZE } from "../../lib/overlay-size";
  import { overlayUrl, panel } from "../panel.svelte";

  const ALIGNS: { value: Align; label: string; icon: typeof AlignStartIcon }[] = [
    { value: "left", label: "左寄せ", icon: AlignStartIcon },
    { value: "center", label: "中央", icon: AlignCenterIcon },
    { value: "right", label: "右寄せ", icon: AlignEndIcon },
  ];
  /** 文字の大きさ（px）。既定を一番大きくする。もっと大きくしたいときは「数値で指定」 */
  const SIZES = [
    { field: "fontSize", label: "話した言葉の大きさ", presets: [16, 24, 32, 40] },
    { field: "translationFontSize", label: "翻訳の大きさ", presets: [16, 20, 26] },
  ] as const;
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
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>字幕オーバーレイ</Card.Title>
      <Card.Description
        >ここで変えた見せ方は、OBS に表示中の字幕にもそのまま反映されます</Card.Description
      >
      <Card.Action>
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
        {#each SIZES as f (f.field)}
          <Field.Field>
            <div class="flex items-center justify-between">
              <Field.Label>{f.label}</Field.Label>
              <FontSizeDialog field={f.field} label={f.label} />
            </div>
            <!-- 数値で指定して選択肢から外れたら、どれも選ばない -->
            <ToggleGroup.Root
              type="single"
              variant="outline"
              class="w-full"
              value={String(d[f.field])}
              onValueChange={(v) => v && set({ [f.field]: Number(v) })}
            >
              {#each f.presets as px (px)}
                <ToggleGroup.Item value={String(px)} class="flex-1">{px}px</ToggleGroup.Item>
              {/each}
            </ToggleGroup.Root>
            {#if !(f.presets as readonly number[]).includes(d[f.field])}
              <Field.Description>数値で指定しています（{d[f.field]}px）</Field.Description>
            {/if}
          </Field.Field>
        {/each}
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
          <div class="flex items-center justify-between">
            <Field.Label>消すまでの時間</Field.Label>
            <DisplayTimingDialog />
          </div>
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
        字幕は箱の下から積み上がり、箱の幅で折り返します。大きさを変えるときは、ソースを引き伸ばさずにプロパティの幅と高さを変えてください（引き伸ばすと文字ごと伸びます）。一度追加すれば、見せ方を変えても貼り直す必要はありません
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <AddToObs url={overlayUrl("subtitles")} name="字幕" size={SUBTITLE_SIZE} />
    </Card.Content>
  </Card.Root>
</div>
