# frozen_string_literal: false
#
# meshtard: SketchUp 2026 trial capture script.
#
# Run in SketchUp (Windows > Ruby Console):
#   load 'C:/path/to/meshtard/tools/trial/capture.rb'
# or paste the whole file into the console.
#
# Writes everything into Desktop/meshtard_trial_<timestamp>/ (see tools/trial/README.md):
#   corpus/2026/..., corpus/2017/...  sample .skp files, same layout as tests/corpus/
#   dumps/                            text and JSON dumps (shortcuts, options, ...)
#   variants/<name>.json              model state of each sample just before it was saved
#   log.txt, results.json, README.txt
#
# Uses only the documented SketchUp Ruby API. Each step is wrapped in begin/rescue, so one
# failure is logged and the rest goes on. Steps run one after another from UI timers, so
# SketchUp stays responsive and asynchronous actions (send_action) settle before a save.
# A few samples need a manual click (no API exists); the script stops there and prints what to
# do, then you type  MeshtardTrial.save_as  in the console to save and go on
# (MeshtardTrial.skip skips one).

require 'fileutils'
begin
  require 'json'
rescue LoadError
  nil
end

module MeshtardTrial
  extend self

  STEP_DELAY = 0.4
  BOX = 100.0 # inches

  # send_action names documented on ruby.sketchup.com (Sketchup.send_action). Listed for
  # reference only: the script never sends most of them.
  DOCUMENTED_ACTIONS = %w[
    showRubyPanel: viewBack: viewBottom: viewFront: viewIso: viewLeft: viewRight: viewTop:
    viewPerspective: viewShowAxes: viewShowHidden: viewZoomExtents: viewZoomToSelection:
    viewUndo: selectOrbitTool: selectPositionCameraTool: selectDollyTool: selectTurnTool:
    selectWalkTool: selectZoomTool: selectFieldOfViewTool: selectZoomWindowTool: pageAdd:
    pageDelete: pageUpdate: pageNext: pagePrevious: renderWireframe: renderHiddenLine:
    renderMonochrome: renderShaded: renderTextures: selectArcTool: selectArc3PointTool:
    selectArc3PointPieTool: selectAxisTool: selectCircleTool: selectEraseTool:
    selectFreehandTool: selectLineTool: selectMeasureTool: selectMoveTool: selectOffsetTool:
    selectPaintTool: selectPolygonTool: selectProtractorTool: selectPushPullTool:
    selectRectangleTool: selectRectangle3PointTool: selectRotateTool: selectScaleTool:
    selectSectionPlaneTool: selectTextTool: selectDimensionTool: selectExtrudeTool:
    selectSelectionTool: editUndo: editRedo: editHide: editUnhide: fixNonPlanarFaces:
    getPhotoTexture: selectImageIglooTool: selectNorthTool: showExtensionStore:
  ].freeze

  # ---------------------------------------------------------------- logging, paths

  def log(msg)
    line = "[#{Time.now.strftime('%H:%M:%S')}] #{msg}"
    puts line
    begin
      File.open(File.join(@out, 'log.txt'), 'a') { |f| f.puts(line) } if @out
    rescue StandardError
      nil
    end
  end

  def log_error(what, err)
    @errors << "#{what}: #{err.class}: #{err.message}"
    log("ERROR in #{what}: #{err.class}: #{err.message}")
    (err.backtrace || [])[0, 4].each { |b| log("    #{b}") }
  end

  # Runs a block, logs any error, returns the block's value or nil.
  def safely(what)
    yield
  rescue Exception => e # rubocop:disable Lint/RescueException
    log_error(what, e)
    nil
  end

  def desktop_dir
    home = ENV['USERPROFILE'] || ENV['HOME'] || Dir.pwd
    candidates = []
    candidates << File.join(ENV['OneDrive'], 'Desktop') if ENV['OneDrive']
    candidates << File.join(home, 'Desktop')
    candidates << File.join(home, 'OneDrive', 'Desktop')
    candidates.find { |d| File.directory?(d) } || home
  end

  def same_path?(a, b)
    return false if a.nil? || b.nil? || a.empty? || b.empty?
    File.expand_path(a).tr('\\', '/').downcase == File.expand_path(b).tr('\\', '/').downcase
  end

  def skp_path(version, sub, name)
    File.join(*[@out, 'corpus', version, sub].reject { |s| s.nil? || s.empty? }, "#{name}.skp")
  end

  def write_text(rel, text)
    path = File.join(@out, rel)
    FileUtils.mkdir_p(File.dirname(path))
    File.open(path, 'wb') { |f| f.write(text) }
    path
  end

  # Collects any API collection that has #each (not all of them include Enumerable).
  def items(coll)
    arr = []
    coll.each { |x| arr << x }
    arr
  end

  # Turns API values into JSON-friendly ones.
  def plain(v)
    case v
    when nil, true, false, String, Integer then v
    when Symbol then v.to_s
    when Float then v.finite? ? v : v.to_s
    when Time then { 'utc' => v.getutc.strftime('%Y-%m-%d %H:%M:%S UTC'), 'epoch' => v.to_i }
    when Array then v.map { |x| plain(x) }
    when Hash then v.each_with_object({}) { |(k, x), h| h[k.to_s] = plain(x) }
    else
      if defined?(Length) && v.is_a?(Length) then v.to_f
      elsif defined?(Sketchup::Color) && v.is_a?(Sketchup::Color) then v.to_a
      elsif defined?(Geom::Point3d) && (v.is_a?(Geom::Point3d) || v.is_a?(Geom::Vector3d))
        v.to_a.map(&:to_f)
      elsif v.respond_to?(:name) then "#{v.class}:#{v.name}"
      else v.to_s
      end
    end
  end

  def write_json(rel, data)
    data = plain(data)
    text = defined?(JSON) ? JSON.pretty_generate(data) : data.inspect
    write_text(rel, text)
  end

  # ---------------------------------------------------------------- step queue

  def add_step(label, delay = STEP_DELAY, &blk)
    @queue << [label, delay, blk]
  end

  def schedule(delay)
    @timer = UI.start_timer(delay, false) do
      # A modal dialog inside a non-repeating timer makes it repeat (documented bug).
      safely('stop_timer') { UI.stop_timer(@timer) } if @timer
      run_next
    end
  end

  def run_next
    return if @busy || @paused
    step = @queue.shift
    return finish unless step
    @busy = true
    label, delay, blk = step
    log("step: #{label}")
    safely(label) { blk.call }
    @busy = false
    schedule(delay) unless @paused
  end

  # ---------------------------------------------------------------- model helpers

  # Opens `path` with no unsaved-changes prompt. The current model is closed without saving
  # first (on Windows Model#close does a File > New).
  def open_clean(path)
    m = Sketchup.active_model
    return true if m && same_path?(m.path, path) && !m.modified?
    safely('close model') { m.close(true) } if m
    begin
      Sketchup.open_file(path, with_status: true)
    rescue ArgumentError, TypeError
      Sketchup.open_file(path)
    end
    ok = same_path?(Sketchup.active_model && Sketchup.active_model.path, path)
    raise "open_file did not open #{path}" unless ok
    true
  end

  def find_box_group(m)
    m.entities.grep(Sketchup::Group).first || raise("the box group of base.skp was not found")
  end

  def iso_camera(m)
    target = Geom::Point3d.new(BOX / 2, BOX / 2, BOX / 2)
    eye = target.offset(Geom::Vector3d.new(-1, -1, 0.8165).normalize, BOX * 4)
    m.active_view.camera = Sketchup::Camera.new(eye, target, Z_AXIS, true, 35.0)
    m.active_view.zoom_extents
  end

  def top_camera(m)
    target = Geom::Point3d.new(BOX / 2, BOX / 2, 0)
    eye = Geom::Point3d.new(BOX / 2, BOX / 2, BOX * 10)
    m.active_view.camera = Sketchup::Camera.new(eye, target, Y_AXIS, false)
    m.active_view.zoom_extents
  end

  def add_box_group(ents, origin, size, name = nil)
    g = ents.add_group
    x, y, z = origin
    f = g.entities.add_face([x, y, z], [x + size, y, z], [x + size, y + size, z], [x, y + size, z])
    f.reverse! if f.normal.z < 0
    f.pushpull(size)
    g.name = name if name
    g
  end

  def add_small_component(m, name, at)
    d = m.definitions.add(name)
    f = d.entities.add_face([0, 0, 0], [20, 0, 0], [20, 20, 0], [0, 20, 0])
    f.reverse! if f.normal.z < 0
    f.pushpull(20)
    m.entities.add_instance(d, Geom::Transformation.new(Geom::Point3d.new(*at)))
  end

  def line_style_named(m, wanted)
    ls = m.line_styles
    names = ls.names
    log("line style names: #{names.inspect}")
    name = names.find { |n| n.casecmp(wanted).zero? } ||
           names.find { |n| n !~ /solid/i }
    name ? ls[name] : nil
  end

  # Model state written next to each sample, so a byte diff knows what changed.
  def model_state(m)
    s = {}
    safely('state: path') { s['path'] = m.path; s['modified'] = m.modified? }
    safely('state: rendering_options') do
      ro = {}
      m.rendering_options.each { |k, v| ro[k] = v }
      s['rendering_options'] = ro
    end
    safely('state: shadow_info') do
      si = {}
      m.shadow_info.each { |k, v| si[k] = v }
      s['shadow_info'] = si
    end
    safely('state: camera') { s['camera'] = camera_hash(m.active_view.camera) }
    safely('state: axes') do
      a = m.axes
      s['axes'] = { 'origin' => a.origin, 'axes' => a.axes }
    end
    safely('state: pages') do
      s['pages'] = items(m.pages).map { |p| page_hash(p) }
      s['selected_page'] = m.pages.selected_page && m.pages.selected_page.name
    end
    safely('state: section') do
      sp = m.entities.active_section_plane
      s['active_section_plane'] = sp && { 'plane' => sp.get_plane, 'name' => sp.name }
    end
    safely('state: tags') { s['tags'] = tags_hash(m) }
    safely('state: environments') do
      next unless m.respond_to?(:environments)
      envs = m.environments
      s['environments'] = items(envs).map(&:name)
      s['environment_current'] = envs.current && envs.current.name
    end
    safely('state: entities') { s['entities'] = entities_summary(m.entities, 0) }
    s
  end

  def camera_hash(c)
    h = { 'eye' => c.eye, 'target' => c.target, 'up' => c.up, 'perspective' => c.perspective?,
          'aspect_ratio' => c.aspect_ratio }
    h['fov'] = c.fov if c.perspective?
    h['height'] = c.height unless c.perspective?
    h['is_2d'] = c.is_2d? if c.respond_to?(:is_2d?)
    h['center_2d'] = c.center_2d if c.respond_to?(:center_2d)
    h['scale_2d'] = c.scale_2d if c.respond_to?(:scale_2d)
    h['fov_is_height'] = c.fov_is_height? if c.respond_to?(:fov_is_height?)
    h
  end

  def page_hash(p)
    h = { 'name' => p.name, 'description' => p.description }
    %w[label use_camera? use_axes? use_hidden_layers? use_hidden_geometry? use_hidden_objects?
       use_rendering_options? use_section_planes? use_shadow_info? use_style?
       use_environment? transition_time delay_time include_in_animation?].each do |meth|
      h[meth] = p.send(meth) if p.respond_to?(meth)
    end
    safely('page layers') { h['hidden_layers'] = items(p.layers).map(&:name) }
    safely('page camera') { h['camera'] = camera_hash(p.camera) }
    h
  end

  def tags_hash(m)
    tags = items(m.layers).map do |l|
      t = { 'name' => l.name, 'visible' => l.visible?, 'color' => l.color }
      t['folder'] = l.folder && l.folder.name if l.respond_to?(:folder)
      t['line_style'] = l.line_style && l.line_style.name if l.respond_to?(:line_style)
      t
    end
    folders = m.layers.respond_to?(:folders) ? m.layers.folders.map(&:name) : nil
    { 'tags' => tags, 'folders' => folders }
  end

  def entities_summary(ents, depth)
    items(ents).map do |e|
      h = { 'type' => e.typename }
      h['tag'] = e.layer.name if e.respond_to?(:layer) && e.layer
      h['locked'] = e.locked? if e.respond_to?(:locked?)
      h['name'] = e.name if e.respond_to?(:name) && !e.is_a?(Sketchup::Text)
      h['text'] = e.text if e.respond_to?(:text)
      if e.is_a?(Sketchup::Edge) && e.curve
        h['curve'] = e.curve.typename
        h['is_polygon'] = e.curve.is_polygon? if e.curve.respond_to?(:is_polygon?)
      end
      if depth < 2 && (e.is_a?(Sketchup::Group) || e.is_a?(Sketchup::ComponentInstance))
        h['entities'] = entities_summary(e.definition.entities, depth + 1)
      end
      h
    end.uniq
  end

  # ---------------------------------------------------------------- saving

  # Saves the active model as <name>.skp twice: current version (2026) and SketchUp 2017.
  def save_pair(name, sub = 'w39')
    m = Sketchup.active_model
    p26 = skp_path('2026', sub, name)
    p17 = skp_path('2017', sub, name)
    FileUtils.mkdir_p(File.dirname(p26))
    FileUtils.mkdir_p(File.dirname(p17))
    safely("#{name}: state") { write_json(File.join('variants', "#{name}.json"), model_state(m)) }
    r = { 'name' => name, 'dir' => sub }
    r['2026'] = safely("#{name}: save 2026") { m.save(p26) } ? true : false
    r['2017'] = safely("#{name}: save 2017") do
      if m.respond_to?(:save_copy)
        m.save_copy(p17, Sketchup::Model::VERSION_2017)
      else
        m.save(p17, Sketchup::Model::VERSION_2017) && m.save(p26)
      end
    end ? true : false
    r['ok'] = r['2026'] && r['2017'] && File.exist?(p26) && File.exist?(p17)
    @results << r
    log("saved #{name}: 2026=#{r['2026']} 2017=#{r['2017']}")
    r['ok']
  end

  # One sample: open base.skp, change one thing, save. `wait` = seconds between the change
  # and the save (for send_action, which runs later).
  def variant(name, from: 'base', wait: STEP_DELAY, &change)
    add_step("#{name}: open #{from}") do
      @variant_ok = false
      open_clean(skp_path('2026', 'w39', from))
      @variant_ok = true
    end
    add_step("#{name}: change", wait) do
      next log("#{name}: skipped, open failed") unless @variant_ok
      m = Sketchup.active_model
      m.start_operation("meshtard #{name}", true)
      begin
        change.call(m)
        m.commit_operation
      rescue Exception => e # rubocop:disable Lint/RescueException
        m.abort_operation
        @variant_ok = false
        raise e
      end
    end
    add_step("#{name}: save") do
      next log("#{name}: NOT saved (an earlier step failed)") unless @variant_ok
      save_pair(name)
    end
  end

  # A sample needing a click in SketchUp: opens `from`, prints `text`, and waits for
  # MeshtardTrial.save_as (or MeshtardTrial.skip).
  def manual(name, text, from: 'base')
    add_step("#{name}: open #{from}") { open_clean(skp_path('2026', 'w39', from)) }
    add_step("#{name}: manual") do
      @paused = true
      @pending = name
      @manual_count += 1
      puts ''
      puts '=' * 72
      puts "MANUAL STEP: #{name}"
      text.each_line { |l| puts "  #{l.rstrip}" }
      puts "Then type:  MeshtardTrial.save_as      (or MeshtardTrial.skip to leave it out)"
      puts '=' * 72
    end
  end

  # Console commands for manual steps.
  def save_as(name = nil)
    name ||= @pending
    raise ArgumentError, 'give a name: MeshtardTrial.save_as("my_sample")' unless name
    save_pair(name)
    resume
  end

  def skip
    log("skipped #{@pending}") if @pending
    @skipped << @pending if @pending
    resume
  end

  def resume
    return unless @paused
    @paused = false
    @pending = nil
    schedule(STEP_DELAY)
  end

  def status
    puts "output: #{@out}"
    puts "queued steps left: #{@queue.size}, paused: #{@paused.inspect} (#{@pending})"
    puts "saved: #{@results.count { |r| r['ok'] }}, errors: #{@errors.size}"
  end

  # ---------------------------------------------------------------- generated assets

  def png_bytes(w, h)
    require 'zlib'
    raw = ''.b
    h.times do |y|
      raw << "\x00".b
      w.times { |x| raw << [x * 255 / (w - 1), y * 255 / (h - 1), 160].pack('C3') }
    end
    chunk = lambda do |type, data|
      [data.bytesize].pack('N') + type + data + [Zlib.crc32(type + data)].pack('N')
    end
    "\x89PNG\r\n\x1a\n".b + chunk.call('IHDR'.b, [w, h, 8, 2, 0, 0, 0].pack('NNC5')) +
      chunk.call('IDAT'.b, Zlib::Deflate.deflate(raw)) + chunk.call('IEND'.b, ''.b)
  end

  # Radiance .hdr, flat (not run-length) pixels, 2:1 so it maps as an equirectangular sky.
  def hdr_bytes(w, h)
    data = "#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y #{h} +X #{w}\n".b
    h.times do |y|
      w.times { |_x| data << (y < h / 2 ? [100, 150, 220, 129] : [140, 120, 90, 128]).pack('C4') }
    end
    data
  end

  def asset(name, bytes)
    path = File.join(@out, 'assets', name)
    FileUtils.mkdir_p(File.dirname(path))
    File.open(path, 'wb') { |f| f.write(bytes) }
    path
  end

  # ---------------------------------------------------------------- dumps

  def dump_all
    m = Sketchup.active_model

    safely('dump version') do
      info = {
        'version' => Sketchup.version, 'version_number' => Sketchup.version_number,
        'is_pro' => (Sketchup.is_pro? rescue nil), 'platform' => (Sketchup.platform rescue nil),
        'locale' => (Sketchup.get_locale rescue nil), 'ruby' => RUBY_VERSION,
        'ruby_platform' => RUBY_PLATFORM, 'template' => (Sketchup.template rescue nil),
        'template_dir' => (Sketchup.template_dir rescue nil),
        'temp_dir' => (Sketchup.temp_dir rescue nil), 'captured_at' => Time.now.to_s,
        'script' => 'meshtard tools/trial/capture.rb'
      }
      write_json('dumps/version.json', info)
    end

    safely('dump shortcuts') do
      list = Sketchup.get_shortcuts
      write_text('dumps/shortcuts.txt', list.join("\n") + "\n")
      log("shortcuts: #{list.size}")
    end

    safely('dump send_action names') do
      write_text('dumps/send_actions_documented.txt',
                 "# Sketchup.send_action names documented on ruby.sketchup.com (not enumerable).\n" +
                 DOCUMENTED_ACTIONS.join("\n") + "\n")
    end

    safely('dump toolbars') do
      lines = UI.toolbar_names.map do |n|
        vis = safely("toolbar_visible? #{n}") { UI.toolbar_visible?(n) }
        "#{n}\t#{vis ? 'visible' : 'hidden'}"
      end
      ruby_tb = []
      safely('ruby toolbars') do
        ObjectSpace.each_object(UI::Toolbar) { |t| ruby_tb << "#{t.name}\t#{t.count} buttons (Ruby)" }
      end
      write_text('dumps/toolbars.txt', "# UI.toolbar_names (native) with visibility\n" +
                 lines.join("\n") + "\n# Ruby (extension) toolbars\n" + ruby_tb.join("\n") + "\n")
    end

    safely('dump rendering options') do
      h = {}
      m.rendering_options.each { |k, v| h[k] = v }
      write_json('dumps/rendering_options.json', h)
      write_json('dumps/rendering_options_keys.json', Sketchup::RenderingOptions.keys)
    end

    safely('dump shadow info') do
      h = {}
      m.shadow_info.each { |k, v| h[k] = v }
      write_json('dumps/shadow_info.json', h)
    end

    safely('dump model options') do
      h = {}
      m.options.each do |prov|
        ph = {}
        safely("options #{prov.name}") { prov.each { |k, v| ph[k] = v } }
        h[prov.name] = ph
      end
      write_json('dumps/model_options.json', h)
      write_json('dumps/units.json', h['UnitsOptions'] || {})
    end

    safely('dump styles') do
      st = m.styles
      list = items(st).map { |s| { 'name' => s.name, 'description' => s.description } }
      write_json('dumps/styles.json', { 'styles' => list,
                                        'selected' => st.selected_style && st.selected_style.name,
                                        'active' => (st.active_style.name rescue nil) })
    end

    safely('dump tags') { write_json('dumps/tags.json', tags_hash(m)) }
    safely('dump line styles') do
      write_json('dumps/line_styles.json', m.respond_to?(:line_styles) ? m.line_styles.names : nil)
    end
    safely('dump environments') do
      next write_json('dumps/environments.json', nil) unless m.respond_to?(:environments)
      envs = m.environments
      write_json('dumps/environments.json',
                 { 'names' => items(envs).map(&:name), 'current' => envs.current && envs.current.name })
    end
    safely('dump camera') { write_json('dumps/camera.json', camera_hash(m.active_view.camera)) }
    safely('dump template model') do
      write_json('dumps/template_model.json',
                 { 'materials' => items(m.materials).map(&:name),
                   'definitions' => items(m.definitions).map(&:name),
                   'entities' => entities_summary(m.entities, 0),
                   'pages' => items(m.pages).map { |p| page_hash(p) },
                   'axes' => (m.axes.to_a rescue nil),
                   'georeferenced' => (m.georeferenced? rescue nil) })
    end
    safely('dump view') do
      v = m.active_view
      h = { 'vpwidth' => v.vpwidth, 'vpheight' => v.vpheight,
            'field_of_view' => v.field_of_view,
            'active_tool' => (m.tools.active_tool_name rescue nil) }
      h['scale_factor'] = (UI.scale_factor(v) rescue (UI.scale_factor rescue nil))
      write_json('dumps/view.json', h)
    end
    safely('dump extensions') do
      list = items(Sketchup.extensions).map do |e|
        { 'name' => e.name, 'version' => e.version, 'creator' => e.creator, 'loaded' => e.loaded? }
      end
      write_json('dumps/extensions.json', list)
    end
    safely('dump read_default probes') do
      # read_default has no documented key list; these are probes, nil = not found.
      probes = [%w[Preferences ShowWelcomeWindow], %w[Preferences FileSaveMinutes],
                %w[Preferences AutoSave], %w[Preferences CreateBackup],
                %w[Preferences UseLargeToolbarIcons], %w[Preferences Units],
                %w[Preferences DefaultTemplate], %w[Preferences ShowInferenceTooltips]]
      h = {}
      probes.each { |sec, key| h["#{sec}/#{key}"] = safely("read_default #{sec}/#{key}") { Sketchup.read_default(sec, key) } }
      write_json('dumps/read_default_probes.json', h)
    end
    safely('copy preference files') do
      roots = [ENV['APPDATA'], ENV['LOCALAPPDATA'],
               File.join(ENV['HOME'] || '', 'Library', 'Application Support')].compact
      n = 0
      roots.each do |root|
        Dir.glob(File.join(root.tr('\\', '/'), 'SketchUp', 'SketchUp 20*', 'SketchUp', '*Preferences*.json')).each do |f|
          dest = File.join(@out, 'dumps', 'preferences', "#{File.basename(File.dirname(File.dirname(f)))}_#{File.basename(f)}".tr(' ', '_'))
          FileUtils.mkdir_p(File.dirname(dest))
          FileUtils.cp(f, dest)
          n += 1
        end
        Dir.glob(File.join(root.tr('\\', '/'), 'SketchUp 20*', 'SketchUp', '*Preferences*.json')).each do |f|
          dest = File.join(@out, 'dumps', 'preferences', "mac_#{File.basename(f)}")
          FileUtils.mkdir_p(File.dirname(dest))
          FileUtils.cp(f, dest)
          n += 1
        end
      end
      log("preference files copied: #{n}")
    end
    safely('copy default template') do
      t = Sketchup.template
      FileUtils.cp(t, File.join(@out, 'dumps', 'default_template.skp')) if t && File.exist?(t)
    end
  end

  # ---------------------------------------------------------------- the samples

  def queue_samples
    add_step('dumps of the default model') { dump_all }
    add_step('empty_default') { save_pair('empty_default', '') }

    add_step('base: build') do
      m = Sketchup.active_model
      m.start_operation('meshtard base', true)
      m.entities.clear!
      m.definitions.purge_unused
      add_box_group(m.entities, [0, 0, 0], BOX, nil)
      iso_camera(m)
      m.commit_operation
    end
    add_step('base: save') { save_pair('base') }

    # 1b scenes
    variant('scene_one') { |m| m.pages.add('A') }
    scenes_two = lambda do |m, props|
      t1 = m.layers.add('T1')
      iso_camera(m)
      a = m.pages.add('A')
      top_camera(m)
      t1.visible = false
      b = m.pages.add('B')
      if props
        b.use_camera = true
        %w[use_axes= use_hidden_layers= use_hidden_geometry= use_hidden_objects=
           use_rendering_options= use_section_planes= use_shadow_info= use_style=
           use_environment=].each { |s| b.send(s, false) if b.respond_to?(s) }
        b.description = 'desc'
      end
      t1.visible = true
      m.pages.selected_page = a
    end
    # Selecting page A runs a scene transition; the save waits for it to end.
    variant('scene_two', wait: 3.0) { |m| scenes_two.call(m, false) }
    variant('scene_props', wait: 3.0) { |m| scenes_two.call(m, true) }

    # face style and edges
    variant('face_wireframe') { |m| m.rendering_options['RenderMode'] = 0 }
    variant('face_hiddenline') { |m| m.rendering_options['RenderMode'] = 1 }
    variant('face_monochrome', wait: 1.5) { |_m| Sketchup.send_action('renderMonochrome:') }
    variant('edges_off') { |m| m.rendering_options['EdgeDisplayMode'] = 0 }

    variant('fog_on') do |m|
      ro = m.rendering_options
      ro['DisplayFog'] = true
      ro['FogUseBkColor'] = false
      ro['FogColor'] = Sketchup::Color.new(255, 0, 0)
      ro['FogStartDist'] = 0.25
      ro['FogEndDist'] = 0.75
    end
    variant('hide_rest') { |m| m.rendering_options['InactiveHidden'] = true }
    variant('hide_similar') { |m| m.rendering_options['InstanceHidden'] = true }

    variant('axes_moved') do |m|
      a = 30.0.degrees
      x = Geom::Vector3d.new(Math.cos(a), Math.sin(a), 0)
      y = Geom::Vector3d.new(-Math.sin(a), Math.cos(a), 0)
      m.axes.set(Geom::Point3d.new(10, 20, 30), x, y, Z_AXIS)
    end

    variant('lock_on') { |m| find_box_group(m).locked = true }
    variant('lock_component_off') { |m| add_small_component(m, 'C1', [150, 0, 0]) }
    variant('lock_component_on') { |m| add_small_component(m, 'C1', [150, 0, 0]).locked = true }

    section = lambda do |m, active|
      sp = m.entities.add_section_plane([BOX / 2, BOX / 2, BOX], [0, 0, -1])
      m.entities.active_section_plane = active ? sp : nil
    end
    variant('section_active') { |m| section.call(m, true) }
    variant('section_inactive') { |m| section.call(m, false) }

    variant('shadows_on') { |m| m.shadow_info['DisplayShadows'] = true }
    variant('shadows_time') do |m|
      si = m.shadow_info
      si['DisplayShadows'] = true
      year = (si['ShadowTime'].year rescue Time.now.year)
      si['ShadowTime'] = Time.utc(year, 7, 3, 9, 15, 0)
      si['Light'] = 60
      si['Dark'] = 30
      si['DisplayOnGroundPlane'] = false
    end

    variant('polygon') { |m| m.entities.add_ngon([200, 50, 0], Z_AXIS, 20, 6) }
    variant('circle6') { |m| m.entities.add_circle([200, 50, 0], Z_AXIS, 20, 6) }

    variant('image') do |m|
      png = asset('image.png', png_bytes(8, 8))
      img = m.entities.add_image(png, [150, 0, 0], 50)
      raise 'add_image returned nil' unless img
    end

    # tags: folders and dashes
    tags = lambda do |m|
      t1 = m.layers.add('T1')
      m.layers.add('T2')
      find_box_group(m).layer = t1
      t1
    end
    variant('tag_base') { |m| tags.call(m) }
    variant('tag_folder') do |m|
      t1 = tags.call(m)
      t1.folder = m.layers.add_folder('F1')
    end
    variant('tag_dashes') do |m|
      t1 = tags.call(m)
      ls = line_style_named(m, 'Dash')
      raise 'no dashed line style found' unless ls
      t1.line_style = ls
    end

    # texts and dimensions: root untagged, root tagged, inside a group and a component
    annot = lambda do |ents, notes, dims, label|
      t = ents.add_text(label, Geom::Point3d.new(BOX, BOX / 2, BOX), Geom::Vector3d.new(30, 0, 30))
      d = ents.add_dimension_linear([0, 0, BOX], [BOX, 0, BOX], [0, -20, 0])
      t.layer = notes if notes
      d.layer = dims if dims
    end
    variant('annot_root') { |m| annot.call(m.entities, nil, nil, 'root') }
    variant('annot_tagged') do |m|
      annot.call(m.entities, m.layers.add('Notes'), m.layers.add('Dims'), 'tagged')
    end
    variant('annot_in_groups') do |m|
      notes = m.layers.add('Notes')
      dims = m.layers.add('Dims')
      edges = m.layers.add('Edges')
      g = find_box_group(m)
      annot.call(g.entities, notes, dims, 'in group')
      g.entities.add_line([0, 0, BOX + 10], [BOX, 0, BOX + 10]).layer = edges
      d = m.definitions.add('NoteComp')
      d.entities.add_line([0, 0, 0], [40, 0, 0])
      d.entities.add_text('in component', Geom::Point3d.new(40, 0, 0), Geom::Vector3d.new(10, 0, 10)).layer = notes
      d.entities.add_dimension_linear([0, 0, 0], [40, 0, 0], [0, -10, 0]).layer = dims
      m.entities.add_instance(d, Geom::Transformation.new(Geom::Point3d.new(150, 0, 0)))
    end
    variant('fonts_dims_base') { |m| annot.call(m.entities, nil, nil, 'fonts') }

    variant('environment') do |m|
      raise 'Model#environments needs SketchUp 2025+' unless m.respond_to?(:environments)
      hdr = asset('sky.hdr', hdr_bytes(16, 8))
      env = m.environments.add('MeshtardSky', hdr)
      m.environments.current = env
    end

    # manual samples (no API): the script stops at each one
    manual('two_point', "Camera > Two-Point Perspective (change nothing else).")
    manual('circle6_converted', "Right-click the 6-segment circle > Convert to Polygon.", from: 'circle6')
    manual('fonts_dims', <<~TXT, from: 'fonts_dims_base')
      Window > Model Info > Text: Screen Text font "Times New Roman" Bold 16 pt.
      Model Info > Dimensions: font "Courier New" 12 pt; Expert Dimension Settings:
      tick "Hide when foreshortened", "Highlight non-associated dimensions" (any color).
      Close Model Info.
    TXT
    manual('section_moved', "Optional: Move the section plane's graphic within its plane (Move tool).\nIf SketchUp does not allow it, type MeshtardTrial.skip", from: 'section_active')
  end

  # ---------------------------------------------------------------- start / finish

  def run
    @out = File.join(desktop_dir, "meshtard_trial_#{Time.now.strftime('%Y%m%d_%H%M%S')}")
    FileUtils.mkdir_p(@out)
    @queue = []
    @results = []
    @errors = []
    @skipped = []
    @paused = false
    @busy = false
    @pending = nil
    @manual_count = 0
    @started = Time.now
    log("meshtard trial capture, SketchUp #{Sketchup.version}, output: #{@out}")
    # File > New with the default template (SketchUp asks to save the current model if needed).
    safely('file_new') { Sketchup.file_new }
    queue_samples
    log("#{@queue.size} steps queued; the console prints each one. Do not click in SketchUp until a MANUAL STEP appears.")
    schedule(1.0)
    nil
  end

  def finish
    secs = (Time.now - @started).round
    ok = @results.select { |r| r['ok'] }.map { |r| r['name'] }
    bad = @results.reject { |r| r['ok'] }.map { |r| r['name'] }
    safely('results.json') do
      write_json('results.json', { 'results' => @results, 'errors' => @errors,
                                   'skipped' => @skipped, 'seconds' => secs,
                                   'sketchup' => Sketchup.version })
    end
    safely('README.txt') { write_text('README.txt', readme_text(ok, bad)) }
    puts ''
    puts '=' * 72
    puts "meshtard trial capture finished in #{secs} s"
    puts "output folder: #{@out}"
    puts "samples saved (2026 + 2017): #{ok.size}  #{ok.join(', ')}"
    puts "samples FAILED: #{bad.join(', ')}" unless bad.empty?
    puts "skipped: #{@skipped.join(', ')}" unless @skipped.empty?
    puts "errors: #{@errors.size} (see log.txt)"
    @errors.each { |e| puts "  #{e}" }
    puts 'Next: README.txt in that folder (what to send back, screenshots to take).'
    puts '=' * 72
    safely('open folder') { UI.openURL('file:///' + @out.tr('\\', '/')) }
  end

  def readme_text(ok, bad)
    <<~TXT
      meshtard SketchUp trial capture
      ===============================
      SketchUp #{Sketchup.version}, captured #{Time.now}

      Send back this whole folder (zip it). Before sending, look through
      dumps/preferences/ (copies of SketchUp's preference files): they can hold
      recent file paths; delete what you do not want to share.

      Where things go in the meshtard repo:
        corpus/2026/*  -> tests/corpus/2026/     corpus/2017/* -> tests/corpus/2017/
        dumps/, variants/, results.json, log.txt -> spec/raw/trial_capture/
        dumps/shortcuts.txt -> also spec/raw/shortcuts.txt

      Saved (each as 2026 and SketchUp 2017): #{ok.join(', ')}
      Failed: #{bad.empty? ? 'none' : bad.join(', ')}
      Skipped: #{@skipped.empty? ? 'none' : @skipped.join(', ')}
      Errors: #{@errors.size} (log.txt)

      Still to do by hand (screenshots / recordings; see tools/trial/README.md in
      the repo for the full list, mapped to TRIAL_CHECKLIST.md):
        S1  Window > Preferences > Shortcuts > Export... -> shortcuts_export.txt   (1)
        S2  every menu and submenu, empty model -> menus/<menu>.png               (3)
        S3  full window, new model, 1920x1080 -> ui/new_model_1920.png            (3)
        S4  View > Toolbars... list and every toolbar -> ui/toolbars/<name>.png   (1c, 3)
        S5  every Default Tray panel expanded -> ui/tray/<panel>.png              (3)
        S6  Window > Styles > Edit, every tab -> ui/styles/<tab>.png              (3)
        S7  Model Info, every page -> model_info/<page>.png                       (4)
        S8  Preferences, every page -> preferences/<page>.png                     (4)
        S9  cursor recording: every tool, empty / face / edge, with modifiers
            -> cursors/cursors.mp4                                                (1c)
        S10 3D view corners and widgets (Solar North on, geo-located)
            -> ui/widgets/*.png                                                   (1d)
        S11 inference locks recording (arrows, Shift, Alt) -> inference/locks.mp4 (1e)
        S12 File > Export / Import dialogs and Options... -> export/, import/     (4)
        S13 Save As dialog version list, with a meshtard-saved file open          (1)
    TXT
  end
end

MeshtardTrial.run
