Pod::Spec.new do |s|
  s.name           = 'ExtendDisplay'
  s.version        = '0.0.0'
  s.summary        = 'extend.computer display, pairing and control bridge'
  s.description    = 'Expo module bridging the extend.computer engine to the iOS app.'
  s.license        = 'UNLICENSED'
  s.author         = 'extend.computer'
  s.homepage       = 'https://extend.computer'
  s.platforms      = { :ios => '16.4' }
  s.swift_version  = '5.9'
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  s.source_files = '*.{h,m,swift}'
  xcconfig = { 'DEFINES_MODULE' => 'YES' }

  # Rust engine built by mobile/scripts/build-engine-ios.sh. Optional until the
  # engine wiring lands so the app still builds without a Rust toolchain.
  if File.exist?(File.join(__dir__, 'ExtendEngine.xcframework'))
    s.vendored_frameworks = 'ExtendEngine.xcframework'
    xcconfig['SWIFT_ACTIVE_COMPILATION_CONDITIONS'] = '$(inherited) EXTEND_ENGINE'
  end
  s.pod_target_xcconfig = xcconfig
end
