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

  # Subfolders are listed explicitly so ExtendEngine.xcframework headers stay out.
  s.source_files = ['*.{h,m,swift}', 'Core/**/*.{h,m,swift}', 'Display/**/*.{h,m,swift}', 'Input/**/*.{h,m,swift}', 'Sharing/**/*.{h,m,swift}']
  xcconfig = { 'DEFINES_MODULE' => 'YES' }

  # Rust engine built by mobile/scripts/build-engine-ios.sh (run by
  # mobile/scripts/build-ios.sh before pod install). Core/ requires it.
  unless File.exist?(File.join(__dir__, 'ExtendEngine.xcframework'))
    raise 'ExtendEngine.xcframework is missing: run mobile/scripts/build-engine-ios.sh first'
  end
  s.vendored_frameworks = 'ExtendEngine.xcframework'
  xcconfig['SWIFT_ACTIVE_COMPILATION_CONDITIONS'] = '$(inherited) EXTEND_ENGINE'

  s.pod_target_xcconfig = xcconfig
end
