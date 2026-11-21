import { env } from "cloudflare:workers"; const get = () => "token"; env.TOKEN = { get };
