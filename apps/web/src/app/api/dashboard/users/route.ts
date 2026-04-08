import { NextResponse } from "next/server";
import { createUser, listUsers } from "@/lib/dashboard";

export async function GET() {
  try {
    const users = await listUsers();
    return NextResponse.json(users);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to load users.";
    return NextResponse.json(
      {
        error: {
          message
        }
      },
      { status: 500 }
    );
  }
}

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as Parameters<typeof createUser>[0];
    const created = await createUser(body);
    return NextResponse.json(created);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to create user.";
    return NextResponse.json(
      {
        error: {
          message
        }
      },
      { status: 400 }
    );
  }
}
