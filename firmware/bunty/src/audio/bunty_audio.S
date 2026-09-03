/*
 * Bunty's greeting, embedded as pre-decoded PCM.
 *
 * Shipping decoded samples keeps playback to a plain I2S write loop instead of
 * pulling in an MP3 decoder. Regenerate after changing bunty_audio.mp3:
 *
 * Paths below are relative to the PlatformIO project dir (firmware/bunty).
 *
 *   ffmpeg -i src/audio/bunty_audio.mp3 -ac 1 -ar 22050 \
 *       -acodec pcm_s16le \
 *       -f s16le src/audio/bunty_audio.pcm -y
 *
 * Keep the rate in step with kAudioSampleRate in main.cpp.
 */
    .section .rodata.bunty_audio, "a", @progbits
    .balign 4

    .global buntyAudioPcm
buntyAudioPcm:
    .incbin "src/audio/bunty_audio.pcm"

    .balign 4
    .global buntyAudioPcmEnd
buntyAudioPcmEnd:
