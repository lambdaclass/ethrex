import { NextResponse } from "next/server";
import { isGoogleAuthConfigured, isNaverAuthConfigured, isKakaoAuthConfigured } from "@/lib/oauth";

export async function GET() {
  return NextResponse.json({
    google: isGoogleAuthConfigured(),
    naver: isNaverAuthConfigured(),
    kakao: isKakaoAuthConfigured(),
  });
}
