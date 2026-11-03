import { mount } from "svelte";
import { fitWidth } from "../../lib/fit-width";
import Items from "./Items.svelte";

// ブラウザソースの既定の幅（操作パネルの ITEMS_SIZE）
fitWidth(460);

mount(Items, { target: document.getElementById("app")! });
