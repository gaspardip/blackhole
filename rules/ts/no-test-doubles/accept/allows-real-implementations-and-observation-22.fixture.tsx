import { env } from "cloudflare:workers"; const factory = { bind() { return native; } }; env.AUTH = factory.bind();
