import * as SDK from "@knocklabs/node"; const { default: Client } = SDK; new Client({ fetch: () => Response.redirect("https://example.test") });
