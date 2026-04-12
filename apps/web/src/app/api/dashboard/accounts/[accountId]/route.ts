import { NextResponse } from "next/server";
import { deleteAccount } from "@/lib/dashboard";

type Params = {
  params: Promise<{
    accountId: string;
  }>;
};

export async function DELETE(_request: Request, context: Params) {
  const { accountId } = await context.params;

  try {
    await deleteAccount(accountId);
    return new NextResponse(null, { status: 204 });
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to delete account.";
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
