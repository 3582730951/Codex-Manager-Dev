import { NextResponse } from "next/server";
import { updateUser } from "@/lib/dashboard";

type Params = {
  params: Promise<{
    userId: string;
  }>;
};

export async function PUT(request: Request, context: Params) {
  const { userId } = await context.params;

  try {
    const body = (await request.json()) as Parameters<typeof updateUser>[1];
    const updated = await updateUser(userId, body);
    return NextResponse.json(updated);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to update user.";
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
