file(READ "${INPUT}" bytes HEX)
string(REGEX REPLACE "(..)" "0x\\1," bytes "${bytes}")
file(WRITE "${OUTPUT}" "#pragma once\nstatic const unsigned char ${SYMBOL}[] = {${bytes}};\n")
