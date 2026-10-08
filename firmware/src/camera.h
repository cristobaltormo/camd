#pragma once

#include <esp_camera.h>

bool cameraStart();
camera_fb_t *cameraFrame();
void cameraRelease(camera_fb_t *frame);
void cameraStop();
bool cameraRunning();
bool cameraControl(const char *name, int value);
void cameraDescribe(int &framesize, int &quality, int &vflip, int &hmirror);
void cameraNoteFrame(bool ok);
bool cameraStuck();
