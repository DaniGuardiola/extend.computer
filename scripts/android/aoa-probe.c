// Read-only Android Open Accessory protocol probe for an explicitly selected
// physical device. No accessory strings, mode changes or interface claims.
// Build: cc -Wall -Wextra -Werror aoa-probe.c $(pkg-config --cflags --libs libusb-1.0) -o aoa-probe
#include <libusb.h>
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "Usage: aoa-probe VENDOR_HEX EXACT_SERIAL\n");
        return 2;
    }
    char *end = NULL;
    unsigned long vendor = strtoul(argv[1], &end, 16);
    if (!argv[1][0] || *end || vendor == 0 || vendor > 65535 ||
        strlen(argv[2]) == 0 || strlen(argv[2]) > 127) {
        fprintf(stderr, "Invalid vendor or serial\n");
        return 2;
    }
    for (const unsigned char *p = (const unsigned char *)argv[2]; *p; ++p) {
        if (!isalnum(*p) && *p != '-' && *p != '_' && *p != '.') {
            fprintf(stderr, "Serial must use letters, digits, dot, dash or underscore\n");
            return 2;
        }
    }
    libusb_context *context = NULL;
    int error = libusb_init(&context);
    if (error < 0) {
        fprintf(stderr, "libusb_init: %s\n", libusb_error_name(error));
        return 1;
    }
    libusb_device **devices = NULL;
    ssize_t count = libusb_get_device_list(context, &devices);
    int result = 1;
    if (count < 0) {
        fprintf(stderr, "libusb_get_device_list: %s\n", libusb_error_name((int)count));
        goto done;
    }
    for (ssize_t i = 0; i < count; ++i) {
        struct libusb_device_descriptor descriptor;
        if (libusb_get_device_descriptor(devices[i], &descriptor) < 0 ||
            descriptor.idVendor != vendor || !descriptor.iSerialNumber) {
            continue;
        }
        libusb_device_handle *handle = NULL;
        error = libusb_open(devices[i], &handle);
        if (error < 0) {
            fprintf(stderr, "Open vendor %04lx device: %s\n", vendor, libusb_error_name(error));
            continue;
        }
        unsigned char serial[256];
        int length = libusb_get_string_descriptor_ascii(handle, descriptor.iSerialNumber,
                                                       serial, sizeof(serial) - 1);
        if (length < 0) {
            fprintf(stderr, "Read serial: %s\n", libusb_error_name(length));
            libusb_close(handle);
            continue;
        }
        serial[length] = 0;
        if (strcmp((const char *)serial, argv[2])) {
            libusb_close(handle);
            continue;
        }
        unsigned char protocol[2] = {0};
        // AOA GET_PROTOCOL (51) is read-only and returns a little-endian u16.
        error = libusb_control_transfer(handle,
            LIBUSB_ENDPOINT_IN | LIBUSB_REQUEST_TYPE_VENDOR | LIBUSB_RECIPIENT_DEVICE,
            51, 0, 0, protocol, sizeof(protocol), 2000);
        if (error == 2) {
            unsigned version = protocol[0] | ((unsigned)protocol[1] << 8);
            printf("{\"serial\":\"%s\",\"vendor\":%u,\"product\":%u,"
                   "\"accessory_protocol\":%u,\"usb_mode_changed\":false}\n",
                   argv[2], descriptor.idVendor, descriptor.idProduct, version);
            result = version ? 0 : 1;
        } else {
            fprintf(stderr, "GET_PROTOCOL for selected device: %s (%d bytes)\n",
                    error < 0 ? libusb_error_name(error) : "Invalid response", error);
        }
        libusb_close(handle);
        break;
    }
    if (result) fprintf(stderr, "Selected accessory-capable device not confirmed\n");
done:
    if (devices) libusb_free_device_list(devices, 1);
    libusb_exit(context);
    return result;
}
