import Knock from "@knocklabs/node"; import * as MSW from "msw"; new Knock({ fetch: () => MSW.HttpResponse.text("ok") });
