import Knock from "@knocklabs/node"; import { HttpResponse as Response } from "msw"; new Knock({ fetch: () => new Response("ok") });
