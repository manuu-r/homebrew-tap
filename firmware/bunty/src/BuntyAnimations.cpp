#include "BuntyAnimations.h"

#include <Fonts/FreeSans9pt7b.h>
#include <Fonts/FreeSansBold18pt7b.h>
#include "flow32/graphics/Display.h"

namespace {

constexpr int16_t kScreenWidth = 240;
constexpr int16_t kScreenHeight = 320;
constexpr int16_t kContentTop = 32;
constexpr int16_t kContentHeight = kScreenHeight - kContentTop;
constexpr int16_t kSpeakingLeft = 22;
constexpr int16_t kSpeakingTop = 97;
constexpr int16_t kSpeakingWidth = 196;
constexpr int16_t kSpeakingHeight = 142;
constexpr uint32_t kSpeakingFrameTimeMs = 105;
constexpr uint32_t kSleepFrameTimeMs = 650;
constexpr uint32_t kWakeFrameTimeMs = 110;

constexpr uint16_t rgb565(uint8_t red, uint8_t green, uint8_t blue) {
  return static_cast<uint16_t>(((red & 0xF8) << 8) |
                               ((green & 0xFC) << 3) | (blue >> 3));
}

constexpr uint16_t kBackground = rgb565(6, 8, 15);
constexpr uint16_t kEyeCyan = rgb565(17, 236, 232);
constexpr uint16_t kEyeGlow = rgb565(5, 62, 72);
constexpr uint16_t kSleepCyan = rgb565(25, 164, 174);
constexpr uint16_t kSleepGlow = rgb565(9, 37, 49);

enum class RoboEyeShape : uint8_t {
  Pill,
  Focused,
  Squint,
  SoftSquare,
  Dot,
  Sleeping,
};

int16_t smaller(int16_t left, int16_t right) {
  return left < right ? left : right;
}

// Flow32's Display provides anti-aliased rounded rectangles on its RGB565
// framebuffer. Masking a few corners turns that one primitive into the same
// solid, geometric eye vocabulary as the reference: pills, angled focus eyes,
// squints, dots, and concave sleeping lids.
void fillRoboEyeShape(Display &display, int16_t centerX, int16_t centerY,
                      int16_t width, int16_t height, bool leftEye,
                      RoboEyeShape shape, uint16_t color) {
  if (width < 2 || height < 2) return;
  const int16_t x = centerX - width / 2;
  const int16_t y = centerY - height / 2;

  if (shape == RoboEyeShape::Dot) {
    display.fillCircle(centerX, centerY, smaller(width, height) / 2, color);
    return;
  }

  int16_t radius = smaller(width, height) / 3;
  if (shape == RoboEyeShape::Squint) radius = height / 2;
  if (shape == RoboEyeShape::SoftSquare) radius = smaller(radius, 10);
  if (shape == RoboEyeShape::Sleeping) radius = smaller(radius, 12);
  if (radius < 2) radius = 2;
  display.fillRoundRect(x, y, width, height, radius, color);

  if (shape == RoboEyeShape::Focused) {
    const int16_t cut = smaller(18, height / 3);
    if (leftEye) {
      display.fillTriangle(centerX - 2, y - 2, x + width + 3, y - 2,
                           x + width + 3, y + cut, kBackground);
    } else {
      display.fillTriangle(x - 3, y - 2, centerX + 2, y - 2, x - 3,
                           y + cut, kBackground);
    }
  } else if (shape == RoboEyeShape::Sleeping) {
    // Carve a soft concave lower edge, leaving a chunky closed eyelid cap.
    const int16_t notchRadius = smaller(width / 2 - 3, height / 2 + 5);
    display.fillCircle(centerX, y + height + 5, notchRadius, kBackground);
  }
}

void drawRoboEye(Display &display, int16_t centerX, int16_t centerY,
                 int16_t width, int16_t height, bool leftEye,
                 RoboEyeShape shape, bool sleeping = false) {
  const uint16_t glow = sleeping ? kSleepGlow : kEyeGlow;
  const uint16_t core = sleeping ? kSleepCyan : kEyeCyan;
  fillRoboEyeShape(display, centerX, centerY, width + 10, height + 10,
                   leftEye, shape, glow);
  fillRoboEyeShape(display, centerX, centerY, width, height, leftEye, shape,
                   core);
}

void drawText(Display &display, const GFXfont *font, const char *text,
              int16_t x, int16_t y, uint16_t color) {
  display.setFont(font);
  display.setTextSize(1);
  display.setTextColor(color);
  display.setTextWrap(false);
  display.setCursor(x, y);
  display.print(text);
}

uint8_t advanceFrame(uint32_t now, uint32_t &lastFrameAt,
                     uint32_t frameTimeMs, uint8_t frame,
                     uint8_t frameCount) {
  const uint32_t elapsed = now - lastFrameAt;
  const uint32_t steps = elapsed / frameTimeMs;
  lastFrameAt += steps * frameTimeMs;
  return static_cast<uint8_t>((frame + steps) % frameCount);
}

}  // namespace

void BuntyAnimations::beginSpeaking(uint32_t now, bool present) {
  frame_ = 0;
  animationStartedAt_ = now;
  lastFrameAt_ = now;
  display_.fillRect(0, kContentTop, kScreenWidth, kContentHeight, kBackground);
  drawSpeakingFrame();
  if (present) {
    display_.present(0, kContentTop, kScreenWidth, kContentHeight);
  }
}

void BuntyAnimations::serviceSpeaking(uint32_t now) {
  if (now - lastFrameAt_ < kSpeakingFrameTimeMs) return;
  frame_ = advanceFrame(now, lastFrameAt_, kSpeakingFrameTimeMs, frame_, 12);
  drawSpeakingFrame();
  display_.present(kSpeakingLeft, kSpeakingTop, kSpeakingWidth,
                   kSpeakingHeight);
}

void BuntyAnimations::drawSpeakingFrame() {
  struct EyePose {
    RoboEyeShape leftShape;
    RoboEyeShape rightShape;
    int8_t leftY;
    int8_t rightY;
    uint8_t leftWidth;
    uint8_t leftHeight;
    uint8_t rightWidth;
    uint8_t rightHeight;
  };
  static constexpr EyePose kPoses[12] = {
      {RoboEyeShape::Pill, RoboEyeShape::Pill, 0, 0, 46, 62, 46, 62},
      {RoboEyeShape::Focused, RoboEyeShape::Focused, -1, -1, 50, 66, 50,
       66},
      {RoboEyeShape::Pill, RoboEyeShape::Pill, -3, 2, 48, 70, 46, 56},
      {RoboEyeShape::Squint, RoboEyeShape::Squint, 1, 1, 58, 22, 58, 22},
      {RoboEyeShape::Pill, RoboEyeShape::SoftSquare, -3, 4, 44, 64, 48, 46},
      {RoboEyeShape::SoftSquare, RoboEyeShape::Dot, 2, 2, 48, 48, 44, 44},
      {RoboEyeShape::Focused, RoboEyeShape::Focused, 0, 0, 52, 68, 52, 68},
      {RoboEyeShape::Pill, RoboEyeShape::Squint, -3, 7, 46, 64, 58, 20},
      {RoboEyeShape::Squint, RoboEyeShape::Pill, 7, -3, 58, 20, 46, 64},
      {RoboEyeShape::Pill, RoboEyeShape::Pill, 1, 1, 52, 54, 52, 54},
      {RoboEyeShape::Squint, RoboEyeShape::Squint, 3, 3, 60, 16, 60, 16},
      {RoboEyeShape::Pill, RoboEyeShape::Pill, 0, 0, 48, 64, 48, 64},
  };

  const EyePose &pose = kPoses[frame_ % 12];
  display_.fillRect(kSpeakingLeft, kSpeakingTop, kSpeakingWidth,
                    kSpeakingHeight, kBackground);

  constexpr int16_t kLeftEyeX = 72;
  constexpr int16_t kRightEyeX = 168;
  constexpr int16_t kEyeY = 167;

  // Solid cyan shapes are the entire speaking character. The pair moves
  // through the reference library's pill, focus, squint, and asymmetric
  // silhouettes without pupils, a mouth, a body, or a greeting label.
  drawRoboEye(display_, kLeftEyeX, kEyeY + pose.leftY, pose.leftWidth,
              pose.leftHeight, true, pose.leftShape);
  drawRoboEye(display_, kRightEyeX, kEyeY + pose.rightY, pose.rightWidth,
              pose.rightHeight, false, pose.rightShape);
}

void BuntyAnimations::beginSleeping(uint32_t now) {
  frame_ = 0;
  animationStartedAt_ = now;
  lastFrameAt_ = now;
  drawSleepFrame();
}

void BuntyAnimations::serviceSleeping(uint32_t now) {
  if (now - lastFrameAt_ < kSleepFrameTimeMs) return;
  frame_ = advanceFrame(now, lastFrameAt_, kSleepFrameTimeMs, frame_, 8);
  drawSleepFrame();
}

void BuntyAnimations::drawSleepFrame() {
  static constexpr int8_t kBreath[8] = {0, -1, -2, -1, 0, 1, 2, 1};
  const uint8_t phase = frame_ % 8;
  const int16_t bob = kBreath[phase];
  display_.fillRect(0, kContentTop, kScreenWidth, kContentHeight, kBackground);

  // Sleep uses the reference's concave closed-eye caps plus a quiet ZZZ.
  const int16_t eyeY = 170 + bob;
  const int16_t breathWidth = 48 + (phase == 2 || phase == 6 ? 2 : 0);
  drawRoboEye(display_, 72, eyeY, breathWidth, 34, true,
              RoboEyeShape::Sleeping, true);
  drawRoboEye(display_, 168, eyeY, breathWidth, 34, false,
              RoboEyeShape::Sleeping, true);

  const int16_t drift = static_cast<int16_t>(phase / 2);
  drawText(display_, &FreeSans9pt7b, "z", 157, 133 - drift, kSleepGlow);
  drawText(display_, &FreeSans9pt7b, "Z", 178, 109 - drift, kSleepCyan);
  drawText(display_, &FreeSansBold18pt7b, "Z", 198, 80 - drift,
           kSleepCyan);
  display_.present(0, kContentTop, kScreenWidth, kContentHeight);
}

void BuntyAnimations::beginWaking(uint32_t now) {
  frame_ = 0;
  animationStartedAt_ = now;
  lastFrameAt_ = now;
  drawWakeFrame();
}

bool BuntyAnimations::serviceWaking(uint32_t now) {
  if (now - animationStartedAt_ >= wakeDurationMs()) return false;
  if (now - lastFrameAt_ >= kWakeFrameTimeMs) {
    frame_ = advanceFrame(now, lastFrameAt_, kWakeFrameTimeMs, frame_, 15);
    drawWakeFrame();
  }
  return true;
}

void BuntyAnimations::drawWakeFrame() {
  static constexpr uint8_t kLeftHeights[15] = {
      8, 8, 12, 20, 32, 46, 60, 70, 64, 68, 64, 62, 64, 64, 64};
  static constexpr uint8_t kRightHeights[15] = {
      8, 8, 8, 14, 26, 40, 56, 68, 62, 70, 66, 62, 64, 64, 64};
  const uint8_t phase = frame_ % 15;
  const int16_t leftHeight = kLeftHeights[phase];
  const int16_t rightHeight = kRightHeights[phase];
  display_.fillRect(0, kContentTop, kScreenWidth, kContentHeight, kBackground);

  constexpr int16_t kLeftEyeX = 72;
  constexpr int16_t kRightEyeX = 168;
  constexpr int16_t kEyeY = 167;
  const RoboEyeShape leftShape = leftHeight <= 20 ? RoboEyeShape::Squint
                                                   : RoboEyeShape::Pill;
  const RoboEyeShape rightShape = rightHeight <= 20 ? RoboEyeShape::Squint
                                                     : RoboEyeShape::Pill;
  drawRoboEye(display_, kLeftEyeX, kEyeY, leftHeight <= 20 ? 58 : 48,
              leftHeight, true, leftShape);
  drawRoboEye(display_, kRightEyeX, kEyeY, rightHeight <= 20 ? 58 : 48,
              rightHeight, false, rightShape);
  display_.present(0, kContentTop, kScreenWidth, kContentHeight);
}
