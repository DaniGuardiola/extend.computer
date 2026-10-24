#include <stdint.h>
#include <stdbool.h>
bool ECDisplaySupported(void);
void *ECDisplayCreate(uint32_t width, uint32_t height, bool retina, uint32_t fps, uint32_t *displayID);
bool ECDisplayConfigure(void *display, uint32_t width, uint32_t height, bool retina, uint32_t fps);
bool ECDisplaySelectMode(void *display, uint32_t width, uint32_t height, bool retina, uint32_t fps);
void ECDisplayDestroy(void *display);
