// 管理画面の Worker の入口（wrangler.admin.jsonc の main。DESIGN.md 17.3）。API の Worker とは別にして、
// Worker ごと Cloudflare Access の後ろに置く（API は誰でも叩けるので、同じ Worker には置けない）

export { admin as default } from "./admin";
