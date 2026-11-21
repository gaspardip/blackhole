import Knock from "@knocklabs/node"; function response() { return Response.json({}); } new Knock({ fetch: async () => response() });
