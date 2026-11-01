import { auth } from "@/lib/auth/server";

// Every sign-in, link and passkey ceremony: Better Auth answers the whole /api/auth tree.
export const dynamic = "force-dynamic";

export const GET = (request: Request) => auth.handler(request);
export const POST = (request: Request) => auth.handler(request);
