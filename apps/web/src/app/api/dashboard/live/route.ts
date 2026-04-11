import { NextResponse } from "next/server";
import { getDashboardLiveSnapshot } from "@/lib/dashboard";

export async function GET() {
  try {
    const snapshot = await getDashboardLiveSnapshot();
    return NextResponse.json(snapshot);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Failed to load live dashboard snapshot.";
    return NextResponse.json(
      {
        error: {
          message
        }
      },
      { status: 502 }
    );
  }
}
