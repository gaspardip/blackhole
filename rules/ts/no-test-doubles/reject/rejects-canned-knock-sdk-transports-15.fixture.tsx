import Knock from "@knocklabs/node"; function options() { return { fetch: () => new Response() }; } new Knock(options());
