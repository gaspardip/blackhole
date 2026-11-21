import { withEnv as scoped } from "cloudflare:workers"; const useEnv = scoped; useEnv({ TOKEN: { get: () => "token" } }, run);
