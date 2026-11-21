import {env} from "cloudflare:workers"; window.Object.assign(window,{fetch:replacement}); window.Object.assign(env,{API:()=>1});
