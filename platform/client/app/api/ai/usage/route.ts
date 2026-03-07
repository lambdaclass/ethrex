import { NextRequest, NextResponse } from "next/server";
import { getUsage } from "@/lib/token-limiter";

export async function GET(req: NextRequest) {
  const deviceId = req.headers.get("x-device-id");
  if (!deviceId || deviceId.length < 10) {
    return NextResponse.json(
      { error: "missing_device_id" },
      { status: 400 }
    );
  }

  const usage = await getUsage(deviceId);
  return NextResponse.json(usage);
}
