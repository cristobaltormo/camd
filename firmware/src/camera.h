#pragma once

bool cameraStart();
void cameraStop();
bool cameraRunning();
bool cameraControl(const char *name, int value);
void cameraDescribe(int &framesize, int &quality, int &vflip, int &hmirror);
void cameraNoteFrame(bool ok);
bool cameraStuck();
