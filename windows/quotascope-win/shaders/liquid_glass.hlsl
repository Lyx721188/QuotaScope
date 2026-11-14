// GPU glass for the Pulse surfaces. Optical ideas adapted from
// WinUI.Composition.Hlsl (MIT), LiquidGlass.hlsl, commit 966c8c8.
//
// The desktop image is blurred by two separable passes before it reaches the
// optical pass. A single 13-tap pass could only reach a few pixels, which left
// desktop text readable through the card. These kernels are wider (sigma is
// roughly ten pixels per pass) but still cheap: thirteen bilinear taps each.
Texture2D source : register(t0);
SamplerState linearClamp : register(s0);

struct VertexOutput { float4 position : SV_Position; float2 uv : TEXCOORD0; };
VertexOutput vs_main(uint id : SV_VertexID) {
    VertexOutput o;
    float2 p = float2((id << 1) & 2, id & 2);
    o.position = float4(p * float2(2, -2) + float2(-1, 1), 0, 1);
    o.uv = p;
    return o;
}

// Gaussian weights for offsets 0, 4, 8, 12, 16, 20, 24 at sigma = 10.
// Normalized so the two passes are a unit-gain filter.
static const float W0 = 0.161003;
static const float W1 = 0.148625;
static const float W2 = 0.116910;
static const float W3 = 0.078369;
static const float W4 = 0.044765;
static const float W5 = 0.021789;
static const float W6 = 0.009038;

float3 blur_horizontal(float2 uv, float2 texel) {
    float2 o = float2(texel.x, 0.0);
    float3 c = source.Sample(linearClamp, uv).rgb * W0;
    c += source.Sample(linearClamp, uv + o * 4).rgb * W1;
    c += source.Sample(linearClamp, uv - o * 4).rgb * W1;
    c += source.Sample(linearClamp, uv + o * 8).rgb * W2;
    c += source.Sample(linearClamp, uv - o * 8).rgb * W2;
    c += source.Sample(linearClamp, uv + o * 12).rgb * W3;
    c += source.Sample(linearClamp, uv - o * 12).rgb * W3;
    c += source.Sample(linearClamp, uv + o * 16).rgb * W4;
    c += source.Sample(linearClamp, uv - o * 16).rgb * W4;
    c += source.Sample(linearClamp, uv + o * 20).rgb * W5;
    c += source.Sample(linearClamp, uv - o * 20).rgb * W5;
    c += source.Sample(linearClamp, uv + o * 24).rgb * W6;
    c += source.Sample(linearClamp, uv - o * 24).rgb * W6;
    return c;
}

float3 blur_vertical(float2 uv, float2 texel) {
    float2 o = float2(0.0, texel.y);
    float3 c = source.Sample(linearClamp, uv).rgb * W0;
    c += source.Sample(linearClamp, uv + o * 4).rgb * W1;
    c += source.Sample(linearClamp, uv - o * 4).rgb * W1;
    c += source.Sample(linearClamp, uv + o * 8).rgb * W2;
    c += source.Sample(linearClamp, uv - o * 8).rgb * W2;
    c += source.Sample(linearClamp, uv + o * 12).rgb * W3;
    c += source.Sample(linearClamp, uv - o * 12).rgb * W3;
    c += source.Sample(linearClamp, uv + o * 16).rgb * W4;
    c += source.Sample(linearClamp, uv - o * 16).rgb * W4;
    c += source.Sample(linearClamp, uv + o * 20).rgb * W5;
    c += source.Sample(linearClamp, uv - o * 20).rgb * W5;
    c += source.Sample(linearClamp, uv + o * 24).rgb * W6;
    c += source.Sample(linearClamp, uv - o * 24).rgb * W6;
    return c;
}

float3 encode_srgb(float3 c) {
    c = saturate(c);
    return lerp(12.92 * c, 1.055 * pow(c, 1.0 / 2.4) - 0.055, step(0.0031308, c));
}

float4 ps_blur_h(VertexOutput input) : SV_Target {
    uint w, h;
    source.GetDimensions(w, h);
    return float4(blur_horizontal(input.uv, 1.0 / float2(w, h)), 1.0);
}

// The HDR desktop arrives as linear scRGB; accumulate in linear light, then
// encode once for the SDR intermediate. The vertical pass works on that SDR
// intermediate, so no second conversion is needed.
float4 ps_blur_h_hdr(VertexOutput input) : SV_Target {
    uint w, h;
    source.GetDimensions(w, h);
    return float4(encode_srgb(blur_horizontal(input.uv, 1.0 / float2(w, h))), 1.0);
}

float4 ps_blur_v(VertexOutput input) : SV_Target {
    uint w, h;
    source.GetDimensions(w, h);
    return float4(blur_vertical(input.uv, 1.0 / float2(w, h)), 1.0);
}

float roundedDistance(float2 p, float2 halfSize, float r) {
    float2 q = abs(p) - (halfSize - r);
    return length(max(q, 0)) + min(max(q.x, q.y), 0) - r;
}

float3 sampled(float2 uv) { return source.Sample(linearClamp, uv).rgb; }

float4 glass(VertexOutput input) {
    uint w, h;
    source.GetDimensions(w, h);
    float2 size = float2(w, h), texel = 1.0 / size;
    float2 p = input.uv * size - size * 0.5;
    float radius = min(min(size.x, size.y) * 0.22, 18.0);
    float d = roundedDistance(p, size * 0.5 - 1.0, radius);
    // Approximate normal of the optical field; Direct2D still clips to the
    // real silhouette, which includes non-rounded-rectangle details.
    float2 n = float2(roundedDistance(p + float2(1, 0), size * 0.5 - 1.0, radius) - d,
                      roundedDistance(p + float2(0, 1), size * 0.5 - 1.0, radius) - d);
    n /= max(length(n), 0.001);
    float edge = 1.0 - smoothstep(0.0, 20.0, max(-d, 0.0));
    // The source is already frosted; moving along the edge bends it a little
    // and splits the channels for a thin, expensive-looking rim.
    float2 shift = -n * edge * 8.0 * texel;
    float2 dispersion = n * edge * 1.6 * texel;
    float3 c = sampled(input.uv + shift);
    c.r = sampled(input.uv + shift - dispersion).r;
    c.b = sampled(input.uv + shift + dispersion).b;
    float glare = edge * pow(saturate(abs(dot(n, normalize(float2(-1, -1))))), 3.0) * 0.06;
    return float4(saturate(c + glare), 1.0);
}

float4 ps_main(VertexOutput input) : SV_Target { return glass(input); }
