import {withEnv} from "cloudflare:workers"; withEnv(await {API:()=>1},run); const transport:typeof fetch=async()=>await new Response();
