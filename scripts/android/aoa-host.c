// Native Android Open Accessory bulk carrier. stdin/stdout contain framed
// encrypted packets only; diagnostics use stderr. Requires exact USB serial.
#include <libusb.h>
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>

static atomic_int stopped;
static int carrier_read = STDIN_FILENO, carrier_write = STDOUT_FILENO;
static void stop_signal(int signal_number) { (void)signal_number; atomic_store(&stopped, 1); }
struct reader_state { libusb_device_handle *handle; unsigned char endpoint; };
static void *receive_bulk(void *input) {
    struct reader_state *state = input;
    unsigned char buffer[16384];
    while (!atomic_load(&stopped)) {
        int count = 0;
        int error = libusb_bulk_transfer(state->handle, state->endpoint, buffer, sizeof(buffer), &count, 100);
        if (error < 0 && error != LIBUSB_ERROR_TIMEOUT) {
            fprintf(stderr, "Accessory read: %s\n", libusb_error_name(error)); atomic_store(&stopped, 1); break;
        }
        size_t offset = 0;
        while (offset < (size_t)count && !atomic_load(&stopped)) {
            struct pollfd ready = { .fd = carrier_write, .events = POLLOUT };
            int available = poll(&ready, 1, 100);
            if (available < 0 && errno == EINTR) continue;
            if (available < 0 || (ready.revents & (POLLERR | POLLHUP))) { atomic_store(&stopped, 1); break; }
            if (available == 0) continue;
            ssize_t written = write(carrier_write, buffer + offset, (size_t)count - offset);
            if (written < 0 && (errno == EINTR || errno == EAGAIN)) continue;
            if (written <= 0) { atomic_store(&stopped, 1); break; }
            offset += (size_t)written;
        }
    }
    return NULL;
}
static libusb_device_handle *selected(libusb_context *context, unsigned vendor, const char *serial, int accessory) {
    libusb_device **devices = NULL;
    ssize_t count = libusb_get_device_list(context, &devices);
    if (count < 0) return NULL;
    libusb_device_handle *matched = NULL;
    for (ssize_t i = 0; i < count; ++i) {
        struct libusb_device_descriptor descriptor;
        if (libusb_get_device_descriptor(devices[i], &descriptor) < 0 || !descriptor.iSerialNumber) continue;
        if (accessory ? (descriptor.idVendor != 0x18d1 || (descriptor.idProduct != 0x2d00 && descriptor.idProduct != 0x2d01)) : descriptor.idVendor != vendor) continue;
        libusb_device_handle *handle = NULL;
        int error = libusb_open(devices[i], &handle);
        if (error < 0) { fprintf(stderr, "Accessory candidate open: %s\n", libusb_error_name(error)); continue; }
        unsigned char actual[256];
        int size = libusb_get_string_descriptor_ascii(handle, descriptor.iSerialNumber, actual, sizeof(actual) - 1);
        if (size < 0) { libusb_close(handle); continue; }
        actual[size] = 0;
        if (!strcmp((const char *)actual, serial)) { matched = handle; break; }
        libusb_close(handle);
    }
    libusb_free_device_list(devices, 1);
    return matched;
}
// Inventory never claims an interface, sets accessory strings, or switches
// USB functions. GET_PROTOCOL (51) only reads accessory support.
static void json_string(const unsigned char *value) {
    putchar('"');
    for (; *value; ++value) {
        if (*value == '"' || *value == '\\') { putchar('\\'); putchar(*value); }
        else if (*value < 32 || *value >= 127) printf("\\u%04x", *value);
        else putchar(*value);
    }
    putchar('"');
}
static int inventory(void) {
    libusb_context *context = NULL;
    if (libusb_init(&context) < 0) return 1;
    libusb_device **devices = NULL;
    ssize_t count = libusb_get_device_list(context, &devices);
    if (count < 0) { libusb_exit(context); return 1; }
    unsigned printed = 0;
    putchar('[');
    for (ssize_t i = 0; i < count && printed < 16; ++i) {
        struct libusb_device_descriptor descriptor;
        if (libusb_get_device_descriptor(devices[i], &descriptor) < 0 || !descriptor.iSerialNumber) continue;
        libusb_device_handle *handle = NULL;
        if (libusb_open(devices[i], &handle) < 0) continue;
        unsigned char serial[128], product[128] = "Android device", protocol[2];
        int size = libusb_get_string_descriptor_ascii(handle, descriptor.iSerialNumber, serial, sizeof(serial) - 1);
        if (size <= 0) { libusb_close(handle); continue; }
        serial[size] = 0;
        int valid = 1;
        for (unsigned char *p = serial; *p; ++p) if (!isalnum(*p) && *p != '-' && *p != '_' && *p != '.') valid = 0;
        if (!valid) { libusb_close(handle); continue; }
        int accessory = descriptor.idVendor == 0x18d1 && (descriptor.idProduct == 0x2d00 || descriptor.idProduct == 0x2d01);
        int supported = accessory || (libusb_control_transfer(handle, 0xc0, 51, 0, 0, protocol, 2, 150) == 2 && (protocol[0] | protocol[1]));
        if (supported) {
            if (descriptor.iProduct) {
                size = libusb_get_string_descriptor_ascii(handle, descriptor.iProduct, product, sizeof(product) - 1);
                if (size > 0) product[size] = 0;
                else strcpy((char *)product, "Android device");
            }
            if (printed++) putchar(',');
            printf("{\"vendor\":\"%04x\",\"serial\":", descriptor.idVendor);
            json_string(serial);
            fputs(",\"name\":", stdout); json_string(product);
            printf(",\"accessory\":%s}", accessory ? "true" : "false");
        }
        libusb_close(handle);
    }
    puts("]");
    libusb_free_device_list(devices, 1);
    libusb_exit(context);
    return 0;
}
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--list")) return inventory();
    if (argc != 4 && argc != 5) { fprintf(stderr, "Usage: aoa-host VENDOR_HEX EXACT_SERIAL HOST_FINGERPRINT [UNIX_SOCKET]\n"); return 2; }
    char *end = NULL; unsigned long vendor = strtoul(argv[1], &end, 16);
    if (!argv[1][0] || *end || vendor == 0 || vendor > 65535 || !argv[2][0] || strlen(argv[2]) > 127 || strlen(argv[3]) != 64) return 2;
    for (const unsigned char *p = (const unsigned char *)argv[2]; *p; ++p) if (!isalnum(*p) && *p != '-' && *p != '_' && *p != '.') return 2;
    for (const unsigned char *p = (const unsigned char *)argv[3]; *p; ++p) if (!isdigit(*p) && (*p < 'a' || *p > 'f')) return 2;
    signal(SIGINT, stop_signal); signal(SIGTERM, stop_signal); signal(SIGPIPE, stop_signal);
    libusb_context *context = NULL; int error = libusb_init(&context); if (error < 0) return 1;
    libusb_device_handle *handle = selected(context, (unsigned)vendor, argv[2], 1);
    if (handle) { fprintf(stderr, "Device already in accessory mode; restore its previous USB function before starting a new host identity\n"); goto failed; }
    if (!handle) {
        handle = selected(context, (unsigned)vendor, argv[2], 0);
        if (!handle) { fprintf(stderr, "Exact USB device unavailable\n"); libusb_exit(context); return 1; }
        unsigned char protocol[2];
        error = libusb_control_transfer(handle, 0xc0, 51, 0, 0, protocol, 2, 2000);
        if (error != 2 || !(protocol[0] | protocol[1])) { fprintf(stderr, "Accessory protocol unavailable\n"); goto failed; }
        const char *strings[] = { "extend.computer", "extend.computer", "Encrypted display transport", "1", "https://extend.computer", argv[3] };
        for (unsigned i = 0; i < 6; ++i) {
            unsigned short length = (unsigned short)(strlen(strings[i]) + 1);
            error = libusb_control_transfer(handle, 0x40, 52, 0, (unsigned short)i, (unsigned char *)strings[i], length, 2000);
            if (error != length) { fprintf(stderr, "Accessory descriptor %u failed: %d\n", i, error); goto failed; }
        }
        error = libusb_control_transfer(handle, 0x40, 53, 0, 0, NULL, 0, 2000);
        if (error < 0 && error != LIBUSB_ERROR_NO_DEVICE) { fprintf(stderr, "Accessory start: %s\n", libusb_error_name(error)); goto failed; }
        libusb_close(handle); handle = NULL;
        for (int attempt = 0; attempt < 200 && !atomic_load(&stopped); ++attempt) {
            usleep(100000); handle = selected(context, (unsigned)vendor, argv[2], 1); if (handle) break;
        }
        if (!handle) { fprintf(stderr, "Accessory did not re-enumerate\n"); libusb_exit(context); return 1; }
    }
    struct libusb_config_descriptor *config = NULL;
    error = libusb_get_active_config_descriptor(libusb_get_device(handle), &config);
    if (error < 0) goto failed;
    int interface = -1; unsigned char in = 0, out = 0;
    for (unsigned i = 0; i < config->bNumInterfaces && interface < 0; ++i) {
        for (int a = 0; a < config->interface[i].num_altsetting && interface < 0; ++a) {
            const struct libusb_interface_descriptor *candidate = &config->interface[i].altsetting[a];
            if (candidate->bInterfaceClass != 255 || candidate->bInterfaceSubClass != 255 || candidate->bAlternateSetting != 0) continue;
            unsigned char found_in = 0, found_out = 0;
            for (unsigned e = 0; e < candidate->bNumEndpoints; ++e) {
                const struct libusb_endpoint_descriptor *endpoint = &candidate->endpoint[e];
                if ((endpoint->bmAttributes & LIBUSB_TRANSFER_TYPE_MASK) != LIBUSB_TRANSFER_TYPE_BULK) continue;
                if (endpoint->bEndpointAddress & LIBUSB_ENDPOINT_IN) found_in = endpoint->bEndpointAddress;
                else found_out = endpoint->bEndpointAddress;
            }
            if (found_in && found_out) { interface = candidate->bInterfaceNumber; in = found_in; out = found_out; }
        }
    }
    libusb_free_config_descriptor(config);
    if (interface < 0) { fprintf(stderr, "Accessory bulk interface unavailable\n"); goto failed; }
    error = libusb_claim_interface(handle, interface);
    if (error < 0) { fprintf(stderr, "Accessory claim: %s\n", libusb_error_name(error)); goto failed; }
    fprintf(stderr, "Accessory bulk ready: serial=%s interface=%d in=%02x out=%02x\n", argv[2], interface, in, out);
    if (argc == 5) {
        struct sockaddr_un address = { .sun_family = AF_UNIX };
        if (strlen(argv[4]) >= sizeof(address.sun_path)) { libusb_release_interface(handle, interface); goto failed; }
        strcpy(address.sun_path, argv[4]);
        int socket_fd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (socket_fd < 0 || connect(socket_fd, (struct sockaddr *)&address, sizeof(address)) < 0) {
            fprintf(stderr, "Native USB worker socket: %s\n", strerror(errno));
            if (socket_fd >= 0) close(socket_fd); libusb_release_interface(handle, interface); goto failed;
        }
        carrier_read = carrier_write = socket_fd;
    }
    int flags = fcntl(carrier_write, F_GETFL);
    if (flags < 0 || fcntl(carrier_write, F_SETFL, flags | O_NONBLOCK) < 0) { libusb_release_interface(handle, interface); goto failed; }
    struct reader_state state = { handle, in }; pthread_t reader;
    if (pthread_create(&reader, NULL, receive_bulk, &state)) { libusb_release_interface(handle, interface); goto failed; }
    unsigned char buffer[16384];
    while (!atomic_load(&stopped)) {
        struct pollfd ready = { .fd = carrier_read, .events = POLLIN };
        int available = poll(&ready, 1, 100);
        if (available < 0 && errno == EINTR) continue;
        if (available < 0) break;
        if (available == 0) continue;
        ssize_t size = read(carrier_read, buffer, sizeof(buffer));
        if (size < 0 && (errno == EINTR || errno == EAGAIN)) continue;
        if (size <= 0) break;
        int offset = 0;
        while (offset < size && !atomic_load(&stopped)) {
            int count = 0;
            error = libusb_bulk_transfer(handle, out, buffer + offset, (int)size - offset, &count, 1000);
            offset += count;
            if (error < 0 && error != LIBUSB_ERROR_TIMEOUT) { fprintf(stderr, "Accessory write: %s\n", libusb_error_name(error)); atomic_store(&stopped, 1); }
        }
    }
    atomic_store(&stopped, 1); pthread_join(reader, NULL);
    if (argc == 5) close(carrier_read);
    libusb_release_interface(handle, interface); libusb_close(handle); libusb_exit(context);
    return 0;
failed:
    if (handle) libusb_close(handle); libusb_exit(context); return 1;
}
