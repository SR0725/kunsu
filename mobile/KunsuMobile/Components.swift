import SwiftUI

// MARK: 來源標記（與桌面 ui/src/agentMarks.ts 同一份向量）

private let claudeSpark = "M52.4285 162.873L98.7844 136.879L99.5485 134.602L98.7844 133.334H96.4921L88.7237 132.862L62.2346 132.153L39.3113 131.207L17.0249 130.026L11.4214 128.844L6.2 121.873L6.7094 118.447L11.4214 115.257L18.171 115.847L33.0711 116.911L55.485 118.447L71.6586 119.392L95.728 121.873H99.5485L100.058 120.337L98.7844 119.392L97.7656 118.447L74.5877 102.732L49.4995 86.1905L36.3823 76.62L29.3779 71.7757L25.8121 67.2858L24.2839 57.3608L30.6515 50.2716L39.3113 50.8623L41.4763 51.4531L50.2636 58.1879L68.9842 72.7209L93.4357 90.6804L97.0015 93.6343L98.4374 92.6652L98.6571 91.9801L97.0015 89.2625L83.757 65.2772L69.621 40.8192L63.2534 30.6579L61.5978 24.632C60.9565 22.1032 60.579 20.0111 60.579 17.4246L67.8381 7.49965L71.9133 6.19995L81.7193 7.49965L85.7946 11.0443L91.9074 24.9865L101.714 46.8451L116.996 76.62L121.453 85.4816L123.873 93.6343L124.764 96.1155H126.292V94.6976L127.566 77.9197L129.858 57.3608L132.15 30.8942L132.915 23.4505L136.608 14.4708L143.994 9.62643L149.725 12.344L154.437 19.0788L153.8 23.4505L150.998 41.6463L145.522 70.1215L141.957 89.2625H143.994L146.414 86.7813L156.093 74.0206L172.266 53.698L179.398 45.6635L187.803 36.802L193.152 32.5484H203.34L210.726 43.6549L207.415 55.1159L196.972 68.3492L188.312 79.5739L175.896 96.2095L168.191 109.585L168.882 110.689L170.738 110.53L198.755 104.504L213.91 101.787L231.994 98.7149L240.144 102.496L241.036 106.395L237.852 114.311L218.495 119.037L195.826 123.645L162.07 131.592L161.696 131.893L162.137 132.547L177.36 133.925L183.855 134.279H199.774L229.447 136.524L237.215 141.605L241.8 147.867L241.036 152.711L229.065 158.737L213.019 154.956L175.45 145.977L162.587 142.787H160.805V143.85L171.502 154.366L191.242 172.089L215.82 195.011L217.094 200.682L213.91 205.172L210.599 204.699L188.949 188.394L180.544 181.069L161.696 165.118H160.422V166.772L164.752 173.152L187.803 207.771L188.949 218.405L187.294 221.832L181.308 223.959L174.813 222.777L161.187 203.754L147.305 182.486L136.098 163.345L134.745 164.2L128.075 235.42L125.019 239.082L117.887 241.8L111.902 237.31L108.718 229.984L111.902 215.452L115.722 196.547L118.779 181.541L121.58 162.873L123.291 156.636L123.14 156.219L121.773 156.449L107.699 175.752L86.304 204.699L69.3663 222.777L65.291 224.431L58.2867 220.768L58.9235 214.27L62.8713 208.48L86.304 178.705L100.44 160.155L109.551 149.507L109.462 147.967L108.959 147.924L46.6977 188.512L35.6182 189.93L30.7788 185.44L31.4156 178.115L33.7079 175.752L52.4285 162.873Z"

struct SourceMark: View {
    let agent: Agent
    var size: CGFloat = 22

    var body: some View {
        mark
            .frame(width: size, height: size)
            .accessibilityLabel(agent.name)
    }

    @ViewBuilder
    private var mark: some View {
        if agent == .claude {
            SVGPath(d: claudeSpark, viewBox: 248).fill(Color(hex: 0xD97757))
        } else {
            Canvas { ctx, sz in SourceMark.drawCodex(&ctx, sz) }
        }
    }

    private static func drawCodex(_ ctx: inout GraphicsContext, _ sz: CGSize) {
        let s: CGFloat = sz.width / 24
        let gradient = Gradient(colors: [Color(hex: 0xB9A8F6), Color(hex: 0x6C78F5), Color(hex: 0x3430EF)])
        var cloud = Path()
        for i in 0..<8 {
            let a: Double = Double(i) / 8 * .pi * 2 + .pi / 8
            let cx: CGFloat = (12 + CGFloat(cos(a)) * 6.2) * s
            let cy: CGFloat = (12 + CGFloat(sin(a)) * 6.2) * s
            let r: CGFloat = 4.6 * s
            cloud.addEllipse(in: CGRect(x: cx - r, y: cy - r, width: r * 2, height: r * 2))
        }
        cloud.addEllipse(in: CGRect(x: 4.8 * s, y: 4.8 * s, width: 14.4 * s, height: 14.4 * s))
        let end = CGPoint(x: sz.width, y: sz.height)
        ctx.fill(cloud, with: .linearGradient(gradient, startPoint: .zero, endPoint: end))
        var prompt = Path()
        prompt.move(to: CGPoint(x: 8.4 * s, y: 9.2 * s))
        prompt.addLine(to: CGPoint(x: 10.9 * s, y: 12 * s))
        prompt.addLine(to: CGPoint(x: 8.4 * s, y: 14.8 * s))
        prompt.move(to: CGPoint(x: 12.6 * s, y: 14.8 * s))
        prompt.addLine(to: CGPoint(x: 16 * s, y: 14.8 * s))
        let style = StrokeStyle(lineWidth: 1.7 * s, lineCap: .round, lineJoin: .round)
        ctx.stroke(prompt, with: .color(Color(hex: 0xF2F3FF)), style: style)
    }
}

/// 軍師（中轉 AI）的頭像：K 圖示，右下角小徽章標出底下用的是 Claude 還是 Codex。
struct RelayAvatar: View {
    let relay: Agent
    var size: CGFloat = 30

    var body: some View {
        Image(uiImage: UIImage(named: "kmark") ?? UIImage())
            .resizable()
            .frame(width: size, height: size)
            .clipShape(RoundedRectangle(cornerRadius: size * 0.28))
            .overlay(alignment: .bottomTrailing) {
                SourceMark(agent: relay, size: size * 0.42)
                    .padding(2)
                    .background(Circle().fill(K.panel))
                    .offset(x: 4, y: 4)
            }
            .accessibilityLabel("軍師（\(relay.name)）")
    }
}

/// 頭像外框：發問的 AI。
struct AgentAvatar: View {
    let agent: Agent
    var body: some View {
        SourceMark(agent: agent, size: 18)
            .frame(width: 30, height: 30)
            .background(Circle().fill(K.panel))
            .overlay(Circle().stroke(K.border))
    }
}
