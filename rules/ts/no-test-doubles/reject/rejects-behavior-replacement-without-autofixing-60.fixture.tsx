import { env } from "cloudflare:workers"; const set = Reflect.set.bind(Reflect, env.AUTH, "verify"); set(() => null);
