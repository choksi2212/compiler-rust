/* Native typed mode: compiler typed_integers.c --std=c11 -o typed.exe */
typedef unsigned long long U64;
typedef int Matrix[3][4];

int main(void) {
    U64 maximum = 18446744073709551615ULL;
    U64 quotient = maximum / 2;
    unsigned int wrapped = 4294967295u;
    wrapped++;
    unsigned char byte = 250;
    byte += 21;
    int count = 0;
    for (U64 i = 0; i < 10; i++) {
        if (i == 2) continue;
        count++;
    }
    /* sizeof does not increment count. */
    return quotient == 9223372036854775807ULL
        && wrapped == 0 && byte == 15 && count == 9
        && sizeof(count++) == 4 && count == 9 && sizeof(Matrix) == 48
        ? 0 : 1;
}
