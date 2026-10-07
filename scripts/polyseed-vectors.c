// Independent Polyseed vectors from the reference implementation, for
// scripts/generate-monero-phrase-vectors.py (build steps there). Usage:
//   harness decode "<phrase>"            -> key hex, birthday, features, lang
//   harness create <lang_en_name> <hex19> <unix_time> [password]
#include "polyseed.h"
#include <CommonCrypto/CommonKeyDerivation.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static unsigned char g_rand[19];
static uint64_t g_time;

static void randbytes(void* result, size_t n) { memcpy(result, g_rand, n); }
static void pbkdf2(const uint8_t* pw, size_t pwlen, const uint8_t* salt, size_t saltlen,
                   uint64_t iterations, uint8_t* key, size_t keylen) {
    CCKeyDerivationPBKDF(kCCPBKDF2, (const char*)pw, pwlen, salt, saltlen, kCCPRFHmacAlgSHA256,
                         (unsigned)iterations, key, keylen);
}
static size_t normalize(const char* str, polyseed_str norm, CFStringNormalizationForm form) {
    CFMutableStringRef s = CFStringCreateMutable(NULL, 0);
    CFStringAppendCString(s, str, kCFStringEncodingUTF8);
    CFStringNormalize(s, form);
    CFStringGetCString(s, norm, POLYSEED_STR_SIZE, kCFStringEncodingUTF8);
    CFRelease(s);
    return strlen(norm);
}
static size_t nfc(const char* str, polyseed_str norm) { return normalize(str, norm, kCFStringNormalizationFormC); }
static size_t nfkd(const char* str, polyseed_str norm) { return normalize(str, norm, kCFStringNormalizationFormKD); }
static uint64_t now(void) { return g_time; }
static void memzero(void* const ptr, const size_t len) { memset(ptr, 0, len); }

static void print_seed(polyseed_data* seed) {
    uint8_t key[32];
    polyseed_keygen(seed, POLYSEED_MONERO, sizeof(key), key);
    printf("key=");
    for (int i = 0; i < 32; i++) printf("%02x", key[i]);
    printf("\nbirthday=%llu\nencrypted=%d\n", (unsigned long long)polyseed_get_birthday(seed),
           polyseed_is_encrypted(seed));
}

int main(int argc, char** argv) {
    polyseed_dependency deps = {0};
    deps.randbytes = randbytes;
    deps.pbkdf2_sha256 = pbkdf2;
    deps.memzero = memzero;
    deps.u8_nfc = nfc;
    deps.u8_nfkd = nfkd;
    deps.time = now;
    polyseed_inject(&deps);
    if (argc >= 3 && strcmp(argv[1], "decode") == 0) {
        polyseed_data* seed;
        const polyseed_lang* lang;
        polyseed_status res = polyseed_decode(argv[2], POLYSEED_MONERO, &lang, &seed);
        printf("status=%d\n", res);
        if (res != POLYSEED_OK) return 0;
        printf("lang=%s\n", polyseed_get_lang_name_en(lang));
        print_seed(seed);
        if (argc >= 4) {
            polyseed_crypt(seed, argv[3]);
            printf("decrypted:\n");
            print_seed(seed);
        }
        polyseed_free(seed);
        return 0;
    }
    if (argc >= 5 && strcmp(argv[1], "create") == 0) {
        for (int i = 0; i < 19; i++) sscanf(argv[3] + 2 * i, "%2hhx", &g_rand[i]);
        g_time = strtoull(argv[4], NULL, 10);
        polyseed_data* seed;
        if (polyseed_create(0, &seed) != POLYSEED_OK) return 1;
        if (argc >= 6) polyseed_crypt(seed, argv[5]);
        const polyseed_lang* lang = NULL;
        for (int i = 0; i < polyseed_get_num_langs(); i++) {
            if (strcmp(polyseed_get_lang_name_en(polyseed_get_lang(i)), argv[2]) == 0) lang = polyseed_get_lang(i);
        }
        if (!lang) return 2;
        polyseed_str phrase;
        polyseed_encode(seed, lang, POLYSEED_MONERO, phrase);
        printf("phrase=%s\n", phrase);
        print_seed(seed);
        polyseed_free(seed);
        return 0;
    }
    return 3;
}
