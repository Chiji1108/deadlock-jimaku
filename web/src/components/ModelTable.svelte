<script lang="ts" module>
  /** 表の列。数字は、行の `cells` の同じ位置にある */
  export interface Column {
    label: string;
    /** 小さいほど良い数字（時間、料金、誤り）。最初に押したとき、小さい順に並べる */
    ascending?: boolean;
    /** 最初の列の数字の、分母（「188 / 189」の 189） */
    outOf?: number;
  }

  export interface Row {
    name: string;
    /** 送り先（測った日が行ごとに違う表では、その日も） */
    via: string;
    note?: string;
    /** サーバーでのこのモデルの名前 */
    server?: string[];
    /** 列ごとの、見せる文字と、並べ替えに使う数字（数字のない欄は、いつも最後に並ぶ） */
    cells: { text: string; value?: number }[];
  }
</script>

<script lang="ts">
  // 測定結果の表（shadcn-svelte の data table）。ビルドのときに HTML にしてあるので、JS が動かなくても読める。
  // JS が動けば、見出しを押して並べ替えられ、「おすすめ」のモードが今使っているモデルに印が付く
  import ArrowDown from "@lucide/svelte/icons/arrow-down";
  import ArrowUp from "@lucide/svelte/icons/arrow-up";
  import ArrowUpDown from "@lucide/svelte/icons/arrow-up-down";
  import {
    createColumnHelper,
    createSortedRowModel,
    createTable,
    rowSortingFeature,
    sortFn_basic,
    sortFn_text,
    tableFeatures,
  } from "@tanstack/svelte-table";
  import { onMount } from "svelte";
  import * as Table from "$lib/components/ui/table";
  import { inUse, type InUse } from "$lib/in-use";

  interface Props {
    /** サーバーでの仕事の名前（`/v2/models` の答えのキー） */
    task: string;
    columns: Column[];
    /** いちばん見てほしい数字（最初の列）の大きい順 */
    rows: Row[];
  }
  // 表の形はページを作るときに決まり、あとから変わらない
  // svelte-ignore state_referenced_locally
  const { task, columns, rows }: Props = $props();

  const features = tableFeatures({ rowSortingFeature, sortedRowModel: createSortedRowModel() });
  const helper = createColumnHelper<typeof features, Row>();
  const table = createTable({
    features,
    data: rows,
    columns: helper.columns([
      helper.accessor("name", { header: "モデル", sortFn: sortFn_text, sortDescFirst: false }),
      ...columns.map((c, i) =>
        helper.accessor((row) => row.cells[i]!.value, {
          id: String(i),
          header: c.label,
          sortFn: sortFn_basic,
          sortDescFirst: !c.ascending,
          sortUndefined: "last",
        }),
      ),
    ]),
    // 同じ数字の行は、渡された順（応答の速い順）のまま
    initialState: { sorting: [{ id: "0", desc: true }] },
    enableSortingRemoval: false,
  });

  let model = $state<InUse[string]>();
  onMount(async () => {
    model = (await inUse())[task];
  });
</script>

<!-- 狭い画面では、見出しを並べ替えのボタンの列に、行を 1 つの箱にして、数字の上に欄の名前を出す -->
<Table.Root class="border-t-2 border-ink max-md:block">
  <Table.Header class="max-md:block">
    <Table.Row class="border-t-0 max-md:flex max-md:flex-wrap max-md:gap-2 max-md:py-4">
      {#each table.getHeaderGroups()[0]!.headers as header, i (header.id)}
        {@const sorted = header.column.getIsSorted()}
        <Table.Head
          aria-sort={sorted ? (sorted === "asc" ? "ascending" : "descending") : undefined}
          class={["max-md:block max-md:py-0", i > 0 && "md:pl-4 md:text-right"]}
        >
          <button
            type="button"
            onclick={header.column.getToggleSortingHandler()}
            class={[
              "inline-flex cursor-pointer items-center gap-1 hover:text-ink max-md:rounded-full max-md:border max-md:border-line max-md:px-3 max-md:py-1.5",
              sorted && "font-bold text-ink max-md:border-ink",
            ]}
          >
            {header.column.columnDef.header}
            {#if sorted === "asc"}
              <ArrowUp class="size-3.5" aria-hidden="true" />
            {:else if sorted === "desc"}
              <ArrowDown class="size-3.5" aria-hidden="true" />
            {:else}
              <ArrowUpDown class="size-3.5 opacity-40" aria-hidden="true" />
            {/if}
          </button>
        </Table.Head>
      {/each}
    </Table.Row>
  </Table.Header>
  <Table.Body class="max-md:block">
    {#each table.getRowModel().rows as { id, original: row } (id)}
      <Table.Row class="align-top max-md:grid max-md:grid-cols-3 max-md:gap-x-4 max-md:gap-y-3 max-md:py-5">
        <Table.Head scope="row" class="whitespace-normal max-md:col-span-3 max-md:py-0">
          <span class="flex flex-wrap items-center gap-x-2 gap-y-1 font-bold">
            {row.name}
            {#if model && row.server?.includes(model.id)}
              <span class="bg-ink px-2 py-0.5 text-xs text-paper">「おすすめ」で使用中</span>
            {/if}
          </span>
          <span class="mt-0.5 block text-sm text-muted">{row.via}</span>
          {#if row.note}
            <span class="mt-1 block max-w-md text-sm leading-relaxed text-muted">{row.note}</span>
          {/if}
        </Table.Head>
        {#each row.cells as cell, i (i)}
          <Table.Cell
            data-label={columns[i]!.label}
            class={[
              "md:pl-4 md:text-right max-md:py-0 max-md:before:block max-md:before:text-xs max-md:before:text-muted max-md:before:content-[attr(data-label)]",
              i === 0 && "font-bold",
            ]}
          >
            {cell.text}
            {#if columns[i]!.outOf}
              <span class="text-sm font-normal text-muted"> / {columns[i]!.outOf}</span>
            {/if}
          </Table.Cell>
        {/each}
      </Table.Row>
    {/each}
  </Table.Body>
</Table.Root>
{#if model && !rows.some((r) => r.server?.includes(model!.id))}
  <p class="mt-4 text-sm text-muted">
    今の「おすすめ」のモードは、この表にない <span class="font-bold text-ink">{model.label}</span> を使っています。
  </p>
{/if}
