<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { RankContent } from "../../generated/RankContent";
  import type { Region } from "../../generated/Region";
  import { REGIONS } from "../../lib/regions";
  import AddToObs from "../parts/AddToObs.svelte";
  import AnchorPicker from "../parts/AnchorPicker.svelte";
  import FontPicker from "../parts/FontPicker.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import StyleNumber from "../parts/StyleNumber.svelte";
  import TextStyleFields from "../parts/TextStyleFields.svelte";
  import { rankSize } from "../../lib/overlay-size";
  import type { MergePatch } from "../../lib/text-style";
  import { cleared, RANK_TEXT } from "../../lib/text-style";
  import type { RankStyle } from "../../generated/RankStyle";
  import { overlayUrl, panel } from "../panel.svelte";

  const CONTENTS: { value: RankContent; label: string }[] = [
    { value: "gauge", label: "名前とゲージ" },
    { value: "full", label: "名前とポイント" },
    { value: "name", label: "名前だけ" },
    { value: "icon", label: "アイコンだけ" },
  ];
  const regions = Object.entries(REGIONS) as [Region, { ja: string }][];
  const NO_REGION = "";

  const s = $derived(panel.settings!);
  const size = $derived(rankSize(s.rankContent));
  // ランク名を取ってくる言語（話す言語・英語・翻訳先）から選ぶ
  const languages = $derived([...new Set([s.micLanguage, "en", ...s.translateTo])]);
  /** 選べる言語にないもの（翻訳先から外した）は、サーバーと同じく話す言語に合わせる */
  const FOLLOW = "";
  const language = $derived(
    s.rankLanguage && languages.includes(s.rankLanguage) ? s.rankLanguage : FOLLOW,
  );

  const setStyle = (patch: MergePatch<RankStyle>) => panel.update({ rankStyle: patch });

  const saved = () => panel.settings?.rankAccount?.toString() ?? "";
  let account = $state(saved());

  /** サーバーは SteamID64 も 32bit の ID に直して保存するので、保存した値を入力欄に戻す */
  async function saveAccount() {
    // 失敗したら直せるよう、入力は残す
    if (await panel.update({ rankAccount: account })) account = saved();
  }
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>ランクオーバーレイ</Card.Title>
      <Card.Description>
        ランクのアイコン・名前・ゲージを出します。試合の結果が反映されるまで少しかかります
      </Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
      <OverlayPreview
        src="/overlay/rank.html?preview"
        title="ランクオーバーレイのプレビュー"
        {size}
      />

      <form
        onsubmit={(e) => {
          e.preventDefault();
          void saveAccount();
        }}
      >
        <Field.Field>
          <Field.Label for="account">Steam のフレンドコード</Field.Label>
          <div class="flex gap-2">
            <Input
              id="account"
              bind:value={account}
              placeholder="フレンドコード"
              inputmode="numeric"
              autocomplete="off"
            />
            <Button type="submit" disabled={account === saved()}>保存</Button>
          </div>
          <Field.Description>
            Steam
            の「フレンド」→「フレンドを追加」に出ている、あなたのフレンドコード（数字）を入れてください。SteamID64（7656…
            で始まる 17 桁）でも使えます。空にすると表示しません
          </Field.Description>
        </Field.Field>
      </form>

      <Field.Field>
        <Field.Label>出すもの</Field.Label>
        <Field.Description>「アイコンだけ」以外は、アイコンの横に出します</Field.Description>
        <ToggleGroup.Root
          type="single"
          variant="outline"
          class="w-full"
          value={s.rankContent}
          onValueChange={(v) => v && panel.update({ rankContent: v as RankContent })}
        >
          {#each CONTENTS as c (c.value)}
            <ToggleGroup.Item value={c.value} class="flex-1">{c.label}</ToggleGroup.Item>
          {/each}
        </ToggleGroup.Root>
      </Field.Field>

      <Field.Field>
        <Field.Label>位置</Field.Label>
        <AnchorPicker
          label="ランクの位置"
          value={s.rankAnchor}
          onchange={(rankAnchor) => panel.update({ rankAnchor })}
        />
        <Field.Description>OBS のソースの箱の中で、ランクを寄せる位置です</Field.Description>
      </Field.Field>

      <Field.Field>
        <Field.Label>エターナスの順位</Field.Label>
        <Select.Root
          type="single"
          value={s.rankRegion ?? NO_REGION}
          onValueChange={(v) => panel.update({ rankRegion: v })}
        >
          <Select.Trigger class="w-full">
            {s.rankRegion ? REGIONS[s.rankRegion].ja : "出さない"}
          </Select.Trigger>
          <Select.Content>
            <Select.Item value={NO_REGION}>出さない</Select.Item>
            {#each regions as [value, name] (value)}
              <Select.Item {value}>{name.ja}のリーダーボード</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
        <Field.Description>
          エターナスのときだけ、ポイントの代わりにリーダーボードの順位（例: アジア
          123位）を出します。更新は 1
          時間ごとです。載っていないときや、同じ名前のプレイヤーがいて見分けられないときはポイントを出します
        </Field.Description>
      </Field.Field>

      <Field.Field>
        <Field.Label>ランク名の言語</Field.Label>
        <Select.Root
          type="single"
          value={language}
          onValueChange={(v) => panel.update({ rankLanguage: v })}
        >
          <Select.Trigger class="w-full">
            {language === FOLLOW
              ? `話す言語に合わせる（${panel.label(s.micLanguage)}）`
              : panel.label(language)}
          </Select.Trigger>
          <Select.Content>
            <Select.Item value={FOLLOW}
              >話す言語に合わせる（{panel.label(s.micLanguage)}）</Select.Item
            >
            {#each languages as code (code)}
              <Select.Item value={code}>{panel.label(code)}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
      </Field.Field>

      <Field.Separator />

      <TextStyleFields
        look={s.rankStyle}
        standard={RANK_TEXT}
        bandHint="明るい場面でも読めるよう、ランクの後ろに帯を敷きます"
        onchange={setStyle}
        onreset={() => setStyle(cleared(s.rankStyle))}
      >
        {#snippet fonts()}
          <FontPicker
            target={{ kind: "rank" }}
            language={language || s.micLanguage}
            label="ランク"
            standardWeight={RANK_TEXT.weight}
          />
        {/snippet}
        {#snippet details()}
          <StyleNumber
            label="字間"
            value={s.rankStyle.letterSpacing}
            standard={RANK_TEXT.letterSpacing}
            presets={[0, 0.02, 0.05, 0.1]}
            percent
            range={{ min: -10, max: 10, step: 1 }}
            onchange={(letterSpacing) => setStyle({ letterSpacing })}
          />
        {/snippet}
      </TextStyleFields>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>OBS に追加する</Card.Title>
      <Card.Description>
        大きさを変えるときは、ソースを引き伸ばさずにプロパティの幅と高さを同じ割合で変えてください。中身がくっきりしたまま大きくなります。出すものを変えたときは、プロパティの幅と高さを
        {size[0]}×{size[1]} に合わせ直してください
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <AddToObs url={overlayUrl("rank")} name="ランク" {size} />
    </Card.Content>
  </Card.Root>
</div>
