"""Decode complementary optical counter rows; reject torn or ambiguous samples."""

def checksum(value):
    crc=0
    for bit in reversed(range(12)):
        feedback=((crc>>3)&1)^((value>>bit)&1)
        crc=(crc<<1)&15
        if feedback:crc^=3
    return crc

def decode(rows, minimum_contrast=60):
    if len(rows)!=2 or any(len(row)!=16 for row in rows):raise ValueError('Expected two rows of 16 cells')
    word=0
    for a,b in zip(*rows):
        if abs(a-b)<minimum_contrast:return None
        word=(word<<1)|int(a>b)
    value=word>>4
    return value if checksum(value)==(word&15) else None
