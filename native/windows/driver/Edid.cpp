/*++

    EDID synthesis for extend.computer virtual monitors.

    Builds a structurally valid 128-byte EDID 1.4 base block with our own (not a
    real vendor's) PNP manufacturer id "XEC", the client's stable serial, the
    requested resolution as the preferred detailed timing, the physical size, and
    the monitor name "extend". The OS uses this for monitor identity (which keys
    its display-configuration database), the friendly name, physical size and
    default scale; the advertised modes come from the driver's
    ParseMonitorDescription / QueryTargetModes callbacks, not from parsing this
    block. This file is original extend.computer code.

--*/

#include "Driver.h"

namespace
{
    // Limits of the 18-byte detailed timing descriptor.
    const UINT kDtdMaxActive = 4095;     // 12-bit active pixels/lines
    const UINT kDtdMaxClock10k = 0xFFFF; // 16-bit pixel clock in 10 kHz units

    // Packs a 3-letter PNP id into the two big-endian EDID manufacturer bytes.
    void PackManufacturer(char a, char b, char c, BYTE* out)
    {
        UINT v = ((UINT)(a - 'A' + 1) << 10) | ((UINT)(b - 'A' + 1) << 5) | (UINT)(c - 'A' + 1);
        out[0] = (BYTE)(v >> 8);
        out[1] = (BYTE)(v & 0xFF);
    }

    // Writes an 18-byte display descriptor with up to 13 ASCII bytes, a 0x0A
    // terminator and 0x20 padding (the EDID string-descriptor convention).
    void WriteStringDescriptor(BYTE* p, BYTE tag, const char* text)
    {
        p[0] = 0; p[1] = 0; p[2] = 0; p[3] = tag; p[4] = 0;
        char buf[13];
        memset(buf, 0x20, sizeof(buf));
        bool term = false;
        for (int i = 0; i < 13; i++)
        {
            if (!term && text[i] != '\0') buf[i] = text[i];
            else if (!term) { buf[i] = 0x0A; term = true; }
        }
        memcpy(p + 5, buf, 13);
    }

    // Writes the dummy descriptor (tag 0x10) that fills an unused slot.
    void WriteDummyDescriptor(BYTE* p)
    {
        memset(p, 0, 18);
        p[3] = 0x10;
    }

    // Writes the preferred detailed-timing descriptor for Width x Height at
    // RefreshHz with CVT reduced-blanking-style timings (160-pixel horizontal
    // blank, at least 460 us of vertical blank). A DTD tops out at 4095 active
    // pixels and a 655.35 MHz pixel clock; a mode beyond that is described at an
    // integer fraction of its size, or a lower refresh, with the same aspect.
    // That only affects this informational block: Windows takes the real modes
    // from the driver callbacks.
    void WriteDetailedTiming(BYTE* p, UINT w, UINT h, UINT hz, UINT mmW, UINT mmH)
    {
        UINT divisor = 1;
        while (w / divisor > kDtdMaxActive || h / divisor > kDtdMaxActive) divisor++;
        w /= divisor;
        h /= divisor;

        const UINT hblank = 160, hfp = 48, hsync = 32;
        const UINT vfp = 3, vsync = 6;
        // Lines of vertical blank covering 460 us: h * t / (1 - t), t = hz * 460 us.
        unsigned long long t = (unsigned long long)hz * 460; // in millionths
        UINT vblank = (UINT)(((unsigned long long)h * t + (1000000 - t) - 1) / (1000000 - t));
        if (vblank < vfp + vsync + 1) vblank = vfp + vsync + 1;
        if (vblank > 4095) vblank = 4095;

        UINT htotal = w + hblank;
        UINT vtotal = h + vblank;
        unsigned long long frame = (unsigned long long)htotal * vtotal;
        unsigned long long pclk10k = frame * hz / 10000ull;
        if (pclk10k > kDtdMaxClock10k)
        {
            // Too fast for a DTD: describe the highest refresh that fits.
            unsigned long long fit = (unsigned long long)kDtdMaxClock10k * 10000ull / frame;
            pclk10k = frame * (fit ? fit : 1) / 10000ull;
        }
        if (pclk10k == 0) pclk10k = 1;

        if (mmW > 4095) mmW = 4095;
        if (mmH > 4095) mmH = 4095;

        p[0] = (BYTE)(pclk10k & 0xFF);
        p[1] = (BYTE)((pclk10k >> 8) & 0xFF);
        p[2] = (BYTE)(w & 0xFF);
        p[3] = (BYTE)(hblank & 0xFF);
        p[4] = (BYTE)((((w >> 8) & 0xF) << 4) | ((hblank >> 8) & 0xF));
        p[5] = (BYTE)(h & 0xFF);
        p[6] = (BYTE)(vblank & 0xFF);
        p[7] = (BYTE)((((h >> 8) & 0xF) << 4) | ((vblank >> 8) & 0xF));
        p[8] = (BYTE)(hfp & 0xFF);
        p[9] = (BYTE)(hsync & 0xFF);
        p[10] = (BYTE)(((vfp & 0xF) << 4) | (vsync & 0xF));
        p[11] = (BYTE)((((hfp >> 8) & 0x3) << 6) | (((hsync >> 8) & 0x3) << 4) |
                       (((vfp >> 4) & 0x3) << 2) | ((vsync >> 4) & 0x3));
        p[12] = (BYTE)(mmW & 0xFF);
        p[13] = (BYTE)(mmH & 0xFF);
        p[14] = (BYTE)((((mmW >> 8) & 0xF) << 4) | ((mmH >> 8) & 0xF));
        p[15] = 0;
        p[16] = 0;
        p[17] = 0x1E; // digital separate sync, H/V positive
    }
}

void ExtendBuildEdid(BYTE out[EXTEND_EDID_SIZE], UINT w, UINT h, UINT hz,
    UINT mmW, UINT mmH, UINT32 serial)
{
    if (mmW == 0 || mmH == 0)
    {
        // Default ~96 DPI physical size from the requested pixel size.
        mmW = (w * 254u) / 960u;
        mmH = (h * 254u) / 960u;
    }

    memset(out, 0, EXTEND_EDID_SIZE);

    // Header.
    static const BYTE header[8] = { 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00 };
    memcpy(out, header, 8);

    PackManufacturer('X', 'E', 'C', out + 8); // our own id, not a real vendor
    out[10] = 0x01; out[11] = 0x00;           // product code 0x0001 (LE)
    out[12] = (BYTE)(serial & 0xFF);          // serial number (LE 32-bit)
    out[13] = (BYTE)((serial >> 8) & 0xFF);
    out[14] = (BYTE)((serial >> 16) & 0xFF);
    out[15] = (BYTE)((serial >> 24) & 0xFF);
    out[16] = 1;    // week of manufacture
    out[17] = 36;   // year = 1990 + 36 = 2026
    out[18] = 1;    // EDID version 1
    out[19] = 4;    // revision 4

    out[20] = 0xA0; // digital input, 8 bits per color, interface undefined
    out[21] = (BYTE)((mmW / 10 > 255) ? 255 : mmW / 10); // horizontal image size, cm
    out[22] = (BYTE)((mmH / 10 > 255) ? 255 : mmH / 10); // vertical image size, cm
    out[23] = 120;  // display gamma = (2.2 * 100) - 100
    out[24] = 0x02; // RGB 4:4:4 only; preferred timing is native; not continuous-frequency

    // Chromaticity (sRGB-ish defaults).
    static const BYTE chroma[10] = { 0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54 };
    memcpy(out + 25, chroma, 10);

    // Established / standard timings unused; modes come from the driver callbacks.
    out[35] = 0; out[36] = 0; out[37] = 0;
    for (int i = 0; i < 8; i++) { out[38 + i * 2] = 0x01; out[38 + i * 2 + 1] = 0x01; }

    // Four 18-byte descriptors: preferred timing, serial string, name, dummy.
    // A range-limits descriptor (0xFD) is optional for a display that is not
    // continuous-frequency (byte 24 bit 0 clear), so the last slot is a dummy.
    WriteDetailedTiming(out + 54, w, h, hz, mmW, mmH);

    char serialText[16];
    // Minimal decimal of the serial, no CRT dependency on _itoa.
    {
        UINT32 v = serial;
        char tmp[11]; int n = 0;
        if (v == 0) tmp[n++] = '0';
        while (v && n < 10) { tmp[n++] = (char)('0' + (v % 10)); v /= 10; }
        for (int i = 0; i < n; i++) serialText[i] = tmp[n - 1 - i];
        serialText[n] = '\0';
    }
    WriteStringDescriptor(out + 72, 0xFF, serialText); // serial number
    WriteStringDescriptor(out + 90, 0xFC, "extend");   // monitor name, <= 13 characters
    WriteDummyDescriptor(out + 108);

    out[126] = 0; // no extension blocks

    // Checksum: the 128 bytes must sum to 0 mod 256.
    UINT sum = 0;
    for (int i = 0; i < 127; i++) sum += out[i];
    out[127] = (BYTE)((256 - (sum & 0xFF)) & 0xFF);
}
