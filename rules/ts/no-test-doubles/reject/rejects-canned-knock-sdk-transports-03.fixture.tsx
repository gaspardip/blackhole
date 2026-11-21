import Knock from "@knocklabs/node"; import { HttpResponse } from "msw"; new Knock({ fetch: () => HttpResponse.json({}) });
