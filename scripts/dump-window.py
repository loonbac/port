#!/usr/bin/env python3
"""Vuelca una captura de ventana como arte ASCII.

Sirve para comprobar si un texto está realmente en la pantalla. No reemplaza a
mirar la ventana: automatiza la pregunta "¿esto se ve o no?" cuando la respuesta
no se puede ver a simple vista.
"""
import struct
import sys
import zlib


def read_png(path):
    with open(path, "rb") as handle:
        data = handle.read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "no es un PNG"
    position = 8
    width = height = depth = color_type = None
    compressed = bytearray()
    while position < len(data):
        (length,) = struct.unpack(">I", data[position : position + 4])
        kind = data[position + 4 : position + 8]
        payload = data[position + 8 : position + 8 + length]
        if kind == b"IHDR":
            width, height, depth, color_type = struct.unpack(">IIBB", payload[:10])
        elif kind == b"IDAT":
            compressed += payload
        elif kind == b"IEND":
            break
        position += 12 + length

    assert depth == 8, f"profundidad {depth} no soportada"
    channels = {0: 1, 2: 3, 4: 2, 6: 4}[color_type]
    raw = zlib.decompress(bytes(compressed))

    stride = width * channels
    rows = []
    previous = bytearray(stride)
    offset = 0
    for _ in range(height):
        filter_type = raw[offset]
        line = bytearray(raw[offset + 1 : offset + 1 + stride])
        offset += 1 + stride
        # Deshacer el filtro PNG (spec 9.2), que es lo que hace la imagen legible.
        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up = previous[i]
            up_left = previous[i - channels] if i >= channels else 0
            if filter_type == 1:
                line[i] = (line[i] + left) & 0xFF
            elif filter_type == 2:
                line[i] = (line[i] + up) & 0xFF
            elif filter_type == 3:
                line[i] = (line[i] + (left + up) // 2) & 0xFF
            elif filter_type == 4:
                estimate = left + up - up_left
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - up_left))
                predictor = (left, up, up_left)[distances.index(min(distances))]
                line[i] = (line[i] + predictor) & 0xFF
        rows.append(bytes(line))
        previous = line
    return width, height, channels, rows


def main():
    path = sys.argv[1]
    top = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    height = int(sys.argv[3]) if len(sys.argv) > 3 else 24
    width, total_height, channels, rows = read_png(path)
    print(f"captura: {width}x{total_height}, {channels} canales")

    shades = " .:-=+*#%@"
    for y in range(top, min(top + height, total_height)):
        line = rows[y]
        chars = []
        for x in range(0, min(width, 260), 2):
            pixel = line[x * channels : x * channels + 3]
            if len(pixel) < 3:
                break
            luminance = (pixel[0] * 30 + pixel[1] * 59 + pixel[2] * 11) // 100
            chars.append(shades[min(len(shades) - 1, luminance * len(shades) // 256)])
        print("".join(chars))


if __name__ == "__main__":
    main()
