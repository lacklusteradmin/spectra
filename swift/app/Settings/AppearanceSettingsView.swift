import SwiftUI

struct AppearanceSettingsView: View {
    @Bindable var preferences: AppUserPreferences

    var body: some View {
        Form {
            Section {
                ForEach(AppearanceMode.allCases) { mode in
                    Button {
                        spectraHaptic(.light)
                        preferences.appearanceMode = mode
                    } label: {
                        HStack {
                            Label(AppLocalization.string(mode.label), systemImage: iconName(for: mode))
                            Spacer()
                            if preferences.appearanceMode == mode {
                                Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(.tint)
                            }
                        }.contentShape(Rectangle())
                    }.foregroundStyle(Color.primary)
                }
            } footer: {
                Text(AppLocalization.string("Controls the color scheme used throughout the app. Defaults to System."))
                    .spectraHintText()
            }
        }.navigationTitle(AppLocalization.string("Appearance")).navigationBarTitleDisplayMode(.inline)
    }

    private func iconName(for mode: AppearanceMode) -> String {
        switch mode {
        case .dark: return "moon.fill"
        case .light: return "sun.max.fill"
        case .system: return "circle.lefthalf.filled"
        }
    }
}
