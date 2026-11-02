<script lang="ts">
  import PlayIcon from "@lucide/svelte/icons/play";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Switch } from "$lib/components/ui/switch";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { SubtitleDisplay } from "../../generated/SubtitleDisplay";
  import CopyUrl from "../parts/CopyUrl.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { overlayUrl, panel } from "../panel.svelte";

  const LINES = [1, 2, 3, 4, 5];
  const HOLDS = [4, 6, 10, 15];

  const s = $derived(panel.settings!);
  const d = $derived(s.subtitleDisplay);
  const set = (patch: Partial<SubtitleDisplay>) =>
    panel.update({ subtitleDisplay: { ...d, ...patch } });
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
      <OverlayPreview src="/overlay/subtitles.html" title="字幕オーバーレイのプレビュー" />

      <Field.Group>
        <Field.Field orientation="horizontal">
          <Field.Content>
            <!-- 「原文」より、実際の言語名の方が何のことか分かりやすい -->
            <Field.Label for="show-source">{panel.label(s.micLanguage)}の字幕を出す</Field.Label>
            <Field.Description>
              あなたが話した言葉をそのまま出します。オフにすると翻訳だけになります（翻訳ができるまで少し遅れて出ます）
            </Field.Description>
          </Field.Content>
          <Switch
            id="show-source"
            checked={d.showSource}
            onCheckedChange={(v) => set({ showSource: v })}
          />
        </Field.Field>
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
        ソースの「＋」→「ブラウザ」で、この URL を貼り付けてください（幅 1920・高さ 1080）。
        一度追加すれば、見せ方を変えても貼り直す必要はありません
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <CopyUrl url={overlayUrl("subtitles")} />
    </Card.Content>
  </Card.Root>
</div>
